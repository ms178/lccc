#!/usr/bin/env python3
"""A return through the thunk costs its spelling and nothing else.

`-mfunction-return=thunk-extern` replaces every `ret` by
`jmp __x86_return_thunk`; the Linux kernel is built that way on x86.  The
thunk returns to the caller with every register a return hands over, so the
jump *is* the function's return, and the code in front of it has no reason to
differ by one instruction from the code in front of a `ret`.

It used to differ by a tenth.  The x86-64 peephole read the jump as a transfer
to a symbol outside the function, register liveness gave up every function
that returned (all of them), and each pass that asks "is this register still
read?" fell back to a rule that says no only inside one straight line of code.
Measured on the benchmark programs at -O2: 15 864 instructions with `ret`,
17 624 with the thunk (+11.1 %, 81 of 87 programs); on i686 +2.6 %.

This gate compiles each program both ways and requires the two listings to be
the same text once the thunk jump is spelled `ret` again.  Any pass that
treats the two returns differently shows up as a diff, in the function and on
the line where it happens.  A second leg links the thunk build against a
`__x86_return_thunk` that is one `ret` and requires the programs to print what
the plain build prints.

    check_return_thunk_parity.py --lccc target/release/lccc [--only NAME]...
                                 [--no-run | --run-all] [--self-test] [--list]

Every program's listings are compared.  Linking and running costs ten times
what a listing does, so by default only `RUN_SET` runs -- the programs whose
thunk builds differed in kind before (a tail call, registers kept across a
call, a merged epilogue) -- and `--run-all` runs them all.

i686 (`-m32`): a callee-pops return (`ret $N`, used for a struct returned in
memory) becomes `popl %ecx; leal N(%esp), %esp; jmp __x86_return_thunk_ecx`
under the thunk -- different instructions by design.  Functions that contain
one are compared by instruction count only, with the sequence counted as the
one `ret $N` it replaces.
"""

import argparse
import concurrent.futures as futures
import os
import pathlib
import re
import shlex
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
PROGRAMS = ROOT / "tests" / "benchmark" / "programs"

# Programs linked and run by default: each took a different path to its return
# under the thunk before this gate existed.
RUN_SET = (
    "ackermann",  # recursion: a return per level
    "binary_trees",  # `jmp free@PLT`: a sibling call in place of the return
    "glibc_memcmp",  # six returns sharing one epilogue
    "hot_web_loop",  # registers kept across a call to a local function
    "libm_round_family",  # wrappers that are one tail call
    "llvm_scalar_facts",  # as hot_web_loop, with a boolean result
)

THUNK_FLAG = "-mfunction-return=thunk-extern"
THUNK_JUMP = "jmp __x86_return_thunk"
THUNK_JUMP_ECX = "jmp __x86_return_thunk_ecx"

# One `ret`, under the name the thunk build jumps to.  File-scope asm keeps
# the compiler from giving it a prologue (or a thunk return of its own).
THUNK_DEFINITION = r"""
__asm__(".text\n"
        ".globl __x86_return_thunk\n"
        ".type __x86_return_thunk, @function\n"
        "__x86_return_thunk:\n"
        "\tret\n"
        ".size __x86_return_thunk, .-__x86_return_thunk\n"
        ".globl __x86_return_thunk_ecx\n"
        ".type __x86_return_thunk_ecx, @function\n"
        "__x86_return_thunk_ecx:\n"
        "\tjmp *%ecx\n"
        ".size __x86_return_thunk_ecx, .-__x86_return_thunk_ecx\n");
"""
THUNK_DEFINITION_64 = THUNK_DEFINITION.replace(
    '".globl __x86_return_thunk_ecx\\n"\n'
    '        ".type __x86_return_thunk_ecx, @function\\n"\n'
    '        "__x86_return_thunk_ecx:\\n"\n'
    '        "\\tjmp *%ecx\\n"\n'
    '        ".size __x86_return_thunk_ecx, .-__x86_return_thunk_ecx\\n"',
    '""',
)

FUNC_LABEL = re.compile(r"^([A-Za-z_][A-Za-z0-9_.$]*):\s*$")
FUNC_TYPE = re.compile(r"^\s*\.type\s+([A-Za-z_][A-Za-z0-9_.$]*)\s*,\s*[@%]function\b")
SIZE_DIRECTIVE = re.compile(r"^\s*\.size\s+([A-Za-z_][A-Za-z0-9_.$]*)\s*,")
INSTRUCTION = re.compile(r"^\s+[a-z]")


def program_flags(source):
    """Extra flags a program needs (`NAME.flags` next to `NAME.c`)."""
    flags = source.with_suffix(".flags")
    if not flags.is_file():
        return []
    return shlex.split(flags.read_text())


def compile_asm(lccc, source, arch, opt, thunk, env):
    cmd = [lccc, arch, opt, "-S", "-o", "-", str(source)] + program_flags(source)
    if thunk:
        cmd.append(THUNK_FLAG)
    proc = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=300)
    if proc.returncode != 0:
        raise RuntimeError(
            f"{' '.join(cmd)}: exit {proc.returncode}\n{proc.stderr.strip()[-2000:]}"
        )
    return proc.stdout


def split_functions(asm):
    """`{name: [lines]}` for every function body, and the text outside them.

    A body runs from the label a `.type NAME, @function` announced to the
    `.size` directive of that name.  Text outside bodies (data, directives
    between functions) is kept in order under the key `None` so that it takes
    part in the comparison too.
    """
    out = {None: []}
    current = None
    functions = set()
    for line in asm.splitlines():
        if current is None:
            match = FUNC_TYPE.match(line)
            if match:
                functions.add(match.group(1))
            match = FUNC_LABEL.match(line)
            if match and match.group(1) in functions:
                current = match.group(1)
                out.setdefault(current, []).append(line)
                continue
            out[None].append(line)
            continue
        out[current].append(line)
        match = SIZE_DIRECTIVE.match(line)
        if match and match.group(1) == current:
            current = None
    return out


def plain_returns(lines):
    """The thunk listing with its plain returns spelled `ret` again."""
    return [
        line[: len(line) - len(line.lstrip())] + "ret" if line.strip() == THUNK_JUMP else line
        for line in lines
    ]


def instruction_count(lines):
    return sum(1 for line in lines if INSTRUCTION.match(line))


def callee_pop_sequences(lines):
    """Instructions the `_ecx` thunk form adds over the `ret $N` it replaces.

    `popl %ecx; leal N(%esp), %esp; jmp __x86_return_thunk_ecx` is three
    instructions where the plain build has one.
    """
    return 2 * sum(1 for line in lines if line.strip() == THUNK_JUMP_ECX)


def compare(name, arch, opt, plain_asm, thunk_asm):
    """Problems (strings) in one program's pair of listings."""
    problems = []
    plain = split_functions(plain_asm)
    thunk = split_functions(thunk_asm)
    if list(plain) != list(thunk):
        only_plain = [f for f in plain if f not in thunk]
        only_thunk = [f for f in thunk if f not in plain]
        problems.append(
            f"{name} {arch} {opt}: the two builds do not define the same functions "
            f"(only with ret: {only_plain}, only with the thunk: {only_thunk})"
        )
        return problems
    for func, body in plain.items():
        other = thunk[func]
        label = func or "<outside functions>"
        if any(line.strip() == THUNK_JUMP_ECX for line in other):
            want = instruction_count(body)
            have = instruction_count(other) - callee_pop_sequences(other)
            if have != want:
                problems.append(
                    f"{name} {arch} {opt}: {label}: {want} instructions with `ret $N`, "
                    f"{have} with the thunk (callee-pops sequences counted as one)"
                )
            continue
        normal = plain_returns(other)
        if normal == body:
            continue
        for index, (a, b) in enumerate(zip(body, normal)):
            if a != b:
                context = "\n".join(
                    f"        ret:   {x}\n        thunk: {y}"
                    for x, y in zip(body[index : index + 4], normal[index : index + 4])
                )
                problems.append(
                    f"{name} {arch} {opt}: {label}: differs from line {index + 1} of the "
                    f"function ({instruction_count(body)} instructions with ret, "
                    f"{instruction_count(normal)} with the thunk)\n{context}"
                )
                break
        else:
            problems.append(
                f"{name} {arch} {opt}: {label}: {len(body)} lines with ret, "
                f"{len(normal)} with the thunk"
            )
    return problems


def run_pair(lccc, source, arch, opt, env, workdir):
    """Link and run both builds; problems when their output differs."""
    definition = THUNK_DEFINITION if arch == "-m32" else THUNK_DEFINITION_64
    thunk_c = pathlib.Path(workdir) / f"thunk{arch}.c"
    if not thunk_c.exists():
        thunk_c.write_text(definition)
    results = []
    for thunk in (False, True):
        exe = pathlib.Path(workdir) / f"{source.stem}{arch}{opt}.{'thunk' if thunk else 'ret'}"
        cmd = [lccc, arch, opt, "-o", str(exe), str(source)] + program_flags(source)
        if thunk:
            cmd += [THUNK_FLAG, str(thunk_c)]
        cmd.append("-lm")
        proc = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=300)
        if proc.returncode != 0:
            return [
                f"{source.stem} {arch} {opt}: link of the {'thunk' if thunk else 'ret'} "
                f"build failed: {proc.stderr.strip()[-600:]}"
            ]
        try:
            run = subprocess.run([str(exe)], capture_output=True, env=env, timeout=120)
        except subprocess.TimeoutExpired:
            return [f"{source.stem} {arch} {opt}: the {'thunk' if thunk else 'ret'} build timed out"]
        # stdout and the exit status are the program's answer; several
        # programs report timings on stderr.
        results.append((run.returncode, run.stdout))
    if results[0] != results[1]:
        return [
            f"{source.stem} {arch} {opt}: the thunk build exits {results[1][0]} and prints "
            f"{results[1][1][:120]!r}; the ret build exits {results[0][0]} and prints "
            f"{results[0][1][:120]!r}"
        ]
    return []


def can_run(lccc, arch, env, workdir):
    """Whether this host links and runs `arch` programs."""
    probe = pathlib.Path(workdir) / f"probe{arch}.c"
    probe.write_text("int main(void) { return 0; }\n")
    exe = pathlib.Path(workdir) / f"probe{arch}"
    proc = subprocess.run(
        [lccc, arch, "-O1", "-o", str(exe), str(probe)], capture_output=True, text=True, env=env
    )
    if proc.returncode != 0:
        return False
    try:
        return subprocess.run([str(exe)], capture_output=True, env=env, timeout=20).returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False


def self_test():
    """The comparison finds what it is there to find."""
    head = ".type f, @function\nf:\n"
    plain = head + "    movl %edi, %eax\n    ret\n    .size f, .-f\n"
    same = head + "    movl %edi, %eax\n    jmp __x86_return_thunk\n    .size f, .-f\n"
    extra = (
        head + "    movl %edi, %edx\n    movl %edx, %eax\n"
        "    jmp __x86_return_thunk\n    .size f, .-f\n"
    )
    ghead = ".type g, @function\ng:\n"
    ecx_plain = ghead + "    movl %ecx, %eax\n    ret $4\n    .size g, .-g\n"
    ecx_thunk = (
        ghead + "    movl %ecx, %eax\n    popl %ecx\n    leal 4(%esp), %esp\n"
        "    jmp __x86_return_thunk_ecx\n    .size g, .-g\n"
    )
    ecx_worse = ecx_thunk.replace("    popl %ecx\n", "    movl %eax, %edx\n    popl %ecx\n")
    cases = [
        ("identical modulo spelling", plain, same, 0),
        ("an instruction the thunk build keeps", plain, extra, 1),
        (
            "a function only one build defines",
            plain,
            same + ".type h, @function\nh:\n    ret\n    .size h, .-h\n",
            1,
        ),
        ("callee-pops return, counted", ecx_plain, ecx_thunk, 0),
        ("callee-pops return, one instruction more", ecx_plain, ecx_worse, 1),
        ("data outside functions", plain + ".long 1\n", same + ".long 2\n", 1),
    ]
    failed = 0
    for what, a, b, want in cases:
        got = len(compare("self-test", "-m64", "-O2", a, b))
        if got != want:
            failed += 1
            print(f"self-test FAILED: {what}: {got} problem(s), expected {want}")
    if failed:
        return 1
    print(f"self-test: {len(cases)} cases ok")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--lccc", default=str(ROOT / "target" / "release" / "lccc"))
    parser.add_argument("--only", action="append", default=[], help="program name (repeatable)")
    parser.add_argument("--arch", action="append", choices=["-m64", "-m32"], default=[])
    parser.add_argument("--opt", action="append", default=[])
    parser.add_argument("--no-run", action="store_true", help="compare listings only")
    parser.add_argument("--run-all", action="store_true", help="run every program, not RUN_SET")
    parser.add_argument("--require-m32", action="store_true", help="fail when -m32 cannot be built")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--self-test-only", action="store_true")
    parser.add_argument("--list", action="store_true", help="print the programs and exit")
    parser.add_argument("-j", "--jobs", type=int, default=os.cpu_count() or 2)
    args = parser.parse_args()

    if args.self_test or args.self_test_only:
        status = self_test()
        if status or args.self_test_only:
            return status

    sources = sorted(PROGRAMS.glob("*.c"))
    if args.only:
        wanted = set(args.only)
        sources = [s for s in sources if s.stem in wanted]
        missing = wanted - {s.stem for s in sources}
        if missing:
            print(f"FAIL: no such program: {', '.join(sorted(missing))}")
            return 1
    if args.list:
        print("\n".join(s.stem for s in sources))
        return 0
    if not sources:
        print(f"FAIL: no programs under {PROGRAMS}")
        return 1
    if not args.only:
        gone = sorted(set(RUN_SET) - {s.stem for s in sources})
        if gone:
            print(f"FAIL: RUN_SET names programs that do not exist: {', '.join(gone)}")
            return 1
    lccc = args.lccc
    if not os.access(lccc, os.X_OK):
        print(f"FAIL: {lccc} is not an executable compiler")
        return 1

    env = dict(os.environ)
    arches = args.arch or ["-m64", "-m32"]
    opts = args.opt or ["-O2", "-Os"]

    problems = []
    notes = []
    compared = 0
    ran = 0
    with tempfile.TemporaryDirectory(prefix="thunk-parity-") as workdir:
        for arch in arches:
            # -m32 needs the 32-bit headers; a host without them cannot
            # preprocess the programs at all.
            try:
                compile_asm(lccc, sources[0], arch, "-O1", False, env)
            except RuntimeError as error:
                if arch == "-m32" and not args.require_m32:
                    notes.append(f"-m32 skipped: the host cannot compile for it ({str(error).splitlines()[-1][:120]})")
                    continue
                problems.append(f"{arch}: {error}")
                continue
            runnable = not args.no_run and can_run(lccc, arch, env, workdir)
            if not args.no_run and not runnable:
                notes.append(f"{arch}: listings compared, programs not run (the host cannot run them)")

            def one(job):
                source, opt = job
                found = []
                try:
                    plain = compile_asm(lccc, source, arch, opt, False, env)
                    thunk = compile_asm(lccc, source, arch, opt, True, env)
                except (RuntimeError, subprocess.TimeoutExpired) as error:
                    return [f"{source.stem} {arch} {opt}: {error}"], 0
                found += compare(source.stem, arch, opt, plain, thunk)
                did_run = 0
                # One optimisation level is enough to show the thunk build
                # runs; the listings carry the rest of the proof.
                if runnable and opt == opts[0] and (args.run_all or source.stem in RUN_SET):
                    found += run_pair(lccc, source, arch, opt, env, workdir)
                    did_run = 1
                return found, did_run

            jobs = [(source, opt) for source in sources for opt in opts]
            with futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
                for found, did_run in pool.map(one, jobs):
                    problems += found
                    compared += 1
                    ran += did_run

    for note in notes:
        print(f"note: {note}")
    if problems:
        for problem in problems[:40]:
            print(f"FAIL: {problem}")
        if len(problems) > 40:
            print(f"... and {len(problems) - 40} more")
        print(f"FAIL: {len(problems)} problem(s) in {compared} listing pairs")
        return 1
    print(
        f"PASS: {compared} listing pairs identical once the thunk jump is spelled `ret` "
        f"({len(sources)} programs, {', '.join(arches)}, {', '.join(opts)}); "
        f"{ran} program pairs run with equal output"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
