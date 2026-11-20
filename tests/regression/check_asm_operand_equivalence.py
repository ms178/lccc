#!/usr/bin/env python3
"""An inline asm statement changes its operands and nothing else.

Each generated function exists twice: once with its memory updates, loads and
exchanges written as extended asm (`incq %0`, `addq %1, %0`, `xchgq %0, %1`,
`btq`, `=@cc` outputs ...), once with the same steps in C.  Around the
statements both keep the same eight locals alive and both fold everything
into one checksum, so a register the compiler believed free across an asm
statement, an operand read from the wrong place or a load deleted because
"nothing reads it" shows up as two different numbers.

Why this exists: on x86-64 a statement that stages at most two operands in
registers of the emitter's choice is no longer a call point (the emitter
takes %rcx and %rdx, which hold no value across an instruction), its inputs
keep their registers, and the peephole reads the statement's text instead of
assuming it reads every register.  Kernel code is made of such statements --
per-CPU accessors, atomics, `preempt_count` -- and before, every value alive
across one of them needed a callee-saved register or a stack slot:

    int f1(int *p, int x) { asm("incl %0" : "+m"(*p)); return x + 1; }

was nine instructions with a push, a frame, a spill and a reload; it is four
now (GCC 16.2: three).  The shapes that stay call points -- three staged
operands, a register clobber, a register-bound operand -- are generated too:
the boundary is where a wrong count would hurt.

    check_asm_operand_equivalence.py --lccc target/release/lccc
        [--gcc gcc-16]      also require the fixture to agree under GCC
        [--pairs N] [--seed S] [--only=-m64:-O2,...] [--self-test] [--keep DIR]

96 pairs by default (about 900 statements); `--pairs 1000` is a soak run.

Rows: -m64 and -m32 (when the host can build and run it) at -O0, -O1, -O2,
-O3 and -Os, and -m64 -O2 with `-mfunction-return=thunk-extern`.  The second
half of the gate holds the code itself to a budget on x86-64 at -O2: the
functions of `SHAPES` must not save a register, must not touch the stack and
must stay within the instruction counts recorded there.
"""

import argparse
import os
import pathlib
import random
import re
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent

PREAMBLE = r"""
#include <stdio.h>
#if defined(__x86_64__)
#define S "q"
#else
#define S "l"
#endif
typedef long word;
typedef unsigned long uword;
#define NOINLINE __attribute__((noinline, noclone))
word g_asm, g_ref;
/* The return thunk of -mfunction-return=thunk-extern: one ret. */
__asm__(".text\n.globl __x86_return_thunk\n.type __x86_return_thunk, @function\n"
        "__x86_return_thunk:\n\tret\n.size __x86_return_thunk, .-__x86_return_thunk\n");
static inline uword mix(uword h, uword v) { return (h ^ v) * 0x9E3779B1ul + (h >> 7); }
"""

LOCALS = ("a", "b", "c", "d", "e", "f", "u", "v")


def steps(rng):
    """One step: (asm statement, C statement).  `G` stands for the global."""
    x, y = rng.sample(LOCALS, 2)
    ptr = rng.choice(("p", "q"))
    imm = rng.choice((1, 3, 7, 100, -5))
    bit = rng.randrange(0, 31)
    kind = rng.choice(
        (
            "inc", "dec", "add_imm_g", "add_reg_m", "add_ri_g", "load", "store", "xchg", "xadd",
            "lea", "neg", "bts", "test_bit", "cmp_cc", "three_regs", "clobber", "specific",
            "c_only", "c_only",
        )
    )
    if kind == "inc":
        return (f'__asm__("inc" S " %0" : "+m"(*{ptr}));', f"(*{ptr})++;")
    if kind == "dec":
        return (f'__asm__("dec" S " %0" : "+m"(G));', "G--;")
    if kind == "add_imm_g":
        return (f'__asm__("add" S " ${imm}, %0" : "+m"(G));', f"G += {imm};")
    if kind == "add_reg_m":
        return (f'__asm__("add" S " %1, %0" : "+m"(*{ptr}) : "r"({x}));', f"*{ptr} += {x};")
    if kind == "add_ri_g":
        return (f'__asm__("add" S " %1, %0" : "+m"(G) : "ri"({x}));', f"G += {x};")
    if kind == "load":
        return (f'__asm__("mov" S " %1, %0" : "=r"({x}) : "m"(*{ptr}));', f"{x} = *{ptr};")
    if kind == "store":
        return (f'__asm__("mov" S " %1, %0" : "=m"(*{ptr}) : "r"({x}));', f"*{ptr} = {x};")
    if kind == "xchg":
        return (
            f'__asm__("xchg" S " %0, %1" : "+r"({x}), "+m"(*{ptr}));',
            f"{{ word t = *{ptr}; *{ptr} = {x}; {x} = t; }}",
        )
    if kind == "xadd":
        return (
            f'__asm__("xadd" S " %0, %1" : "+r"({x}), "+m"(*{ptr}) : : "cc");',
            f"{{ word t = *{ptr}; *{ptr} = (word)((uword)t + (uword){x}); {x} = t; }}",
        )
    if kind == "lea":
        return (
            f'__asm__("lea" S " 3(%1,%1,2), %0" : "=r"({x}) : "r"({y}));',
            f"{x} = (word)((uword){y} * 3 + 3);",
        )
    if kind == "neg":
        return (f'__asm__("neg" S " %0" : "+r"({x}));', f"{x} = (word)(0 - (uword){x});")
    if kind == "bts":
        return (
            f'__asm__("bts" S " ${bit}, %0" : "+m"(*{ptr}) : : "cc");',
            f"*{ptr} = (word)((uword)*{ptr} | (1ul << {bit}));",
        )
    if kind == "test_bit":
        # Three operands of which one is printed: a condition-code output.
        return (
            f'{{ unsigned char o; __asm__("bt" S " ${bit}, %1" : "=@ccc"(o) : "m"(*{ptr})); '
            f"{x} += o; }}",
            f"{x} += (word)(((uword)*{ptr} >> {bit}) & 1);",
        )
    if kind == "cmp_cc":
        return (
            f'{{ unsigned char o; __asm__("cmp" S " %2, %1" : "=@ccb"(o) : "r"({x}), "r"({y})); '
            f"{x} += o; }}",
            f"{x} += (uword){x} < (uword){y};",
        )
    if kind == "three_regs":
        # Three staged operands: stays a call point.
        z = rng.choice([n for n in LOCALS if n not in (x, y)])
        return (
            f'__asm__("lea" S " (%1,%2), %0" : "=r"({x}) : "r"({y}), "r"({z}));',
            f"{x} = (word)((uword){y} + (uword){z});",
        )
    if kind == "clobber":
        # A register clobber shifts the scratch order: stays a call point.
        reg = rng.choice(("cx", "dx", "si", "di"))
        return (
            f'__asm__("xor" S " %%" R "{reg}, %%" R "{reg}\\n\\tadd" S " %1, %0" '
            f': "+m"(*{ptr}) : "r"({x}) : R "{reg}", "cc");',
            f"*{ptr} += {x};",
        )
    if kind == "specific":
        # An operand bound to a register: stays a call point.
        return (
            f'__asm__("add" S " %1, %0" : "+a"({x}) : "d"({y}) : "cc");',
            f"{x} = (word)((uword){x} + (uword){y});",
        )
    op = rng.choice(("+", "^", "-"))
    return (None, f"{x} = (word)((uword){x} {op} ((uword){y} << {rng.randrange(1, 5)}));")


def generate(pairs, seed):
    rng = random.Random(seed)
    out = [PREAMBLE, '#if defined(__x86_64__)\n#define R "r"\n#else\n#define R "e"\n#endif\n']
    for index in range(pairs):
        body = [steps(rng) for _ in range(rng.randrange(4, 12))]
        for flavour, glob in (("asm", "g_asm"), ("ref", "g_ref")):
            lines = [
                f"NOINLINE uword {flavour}_{index}(word a, word b, word c, word d, word *p, word *q)",
                "{",
                "\tword e = a ^ 0x55, f = b + 17, u = c - d, v = (a << 3) ^ b;",
            ]
            for asm_text, c_text in body:
                text = c_text if (flavour == "ref" or asm_text is None) else asm_text
                lines.append("\t" + text.replace("G", glob))
            lines += [
                "\tuword h = 0x1234567;",
                *[f"\th = mix(h, (uword){name});" for name in LOCALS],
                f"\treturn mix(mix(mix(h, (uword)*p), (uword)*q), (uword){glob});",
                "}",
            ]
            out.append("\n".join(lines))
    out.append(
        "int main(void)\n{\n\tint bad = 0;\n"
        + "".join(
            f"\t{{\n\t\tword m1[2] = {{ {11 + i}, {-7 * i} }}, m2[2] = {{ {11 + i}, {-7 * i} }};\n"
            f"\t\tg_asm = g_ref = {1000 + 13 * i};\n"
            f"\t\tuword x = asm_{i}({3 + i}, {-11 + 5 * i}, {i * 97}, {41 - i}, &m1[0], &m1[1]);\n"
            f"\t\tuword y = ref_{i}({3 + i}, {-11 + 5 * i}, {i * 97}, {41 - i}, &m2[0], &m2[1]);\n"
            f"\t\tif (x != y || m1[0] != m2[0] || m1[1] != m2[1] || g_asm != g_ref) {{\n"
            f'\t\t\tprintf("pair {i}: %lx != %lx\\n", x, y);\n\t\t\tbad++;\n\t\t}}\n\t}}\n'
            for i in range(pairs)
        )
        + f'\tprintf("%d of {pairs} pairs differ\\n", bad);\n\treturn bad != 0;\n}}\n'
    )
    return "\n\n".join(out)


# x86-64 -O2: one statement each, and the code it may cost.  `budget` is the
# instruction count of the whole function (GCC 16.2 in the comment); none may
# save a register or touch the stack.
SHAPES = r"""
extern int counter;
int inc_through(int *p, int x) { __asm__("incl %0" : "+m"(*p)); return x + 1; }
int read_through(int *p, int x, int y) { int o; __asm__("movl %1, %0" : "=r"(o) : "m"(*p)); return x + y + o; }
int add_to_global(int x, int y) { __asm__("addl %1, %0" : "+m"(counter) : "ri"(x)); return x * y; }
int exchange(int *p, int x, int y, int z) { __asm__("xchgl %0, %1" : "+r"(x), "+m"(*p)); return x + y + z; }
int lock_or(unsigned long *flags, int x) { __asm__("lock orb $2, %0" : "+m"(*flags) : : "cc"); return x + 1; }
"""
SHAPE_BUDGETS = {
    "inc_through": 4,  # GCC 3
    "read_through": 9,  # GCC 5: the output still travels through a frame slot
    "add_to_global": 4,  # GCC 4
    "exchange": 11,  # GCC 5: as read_through
    "lock_or": 4,  # GCC 3
}
INSTRUCTION = re.compile(r"^\s+[a-z]")


def run(cmd, env=None, timeout=300):
    return subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout)


def build_and_run(compiler, source, exe, flags, env):
    proc = run([compiler, *flags, "-o", str(exe), str(source)], env)
    if proc.returncode != 0:
        return None, f"compile failed: {proc.stderr.strip()[-400:]}"
    try:
        proc = run([str(exe)], env, timeout=120)
    except subprocess.TimeoutExpired:
        return None, "timed out"
    return proc.returncode, proc.stdout.strip().splitlines()[-1:] or [""]


def function_bodies(asm):
    bodies, current = {}, None
    for line in asm.splitlines():
        match = re.match(r"^([A-Za-z_][A-Za-z0-9_]*):\s*$", line)
        if match:
            current = match.group(1)
            bodies[current] = []
        elif current is not None:
            if re.match(r"^\s*\.size\s", line):
                current = None
            else:
                bodies[current].append(line)
    return bodies


def check_shapes(lccc, workdir, env, extra=()):
    """Problems in the code of `SHAPES` at -m64 -O2."""
    source = pathlib.Path(workdir) / "shapes.c"
    source.write_text(SHAPES)
    proc = run([lccc, "-m64", "-O2", "-S", "-o", "-", str(source), *extra], env)
    if proc.returncode != 0:
        return [f"shapes: compile failed: {proc.stderr.strip()[-400:]}"]
    problems = []
    bodies = function_bodies(proc.stdout)
    for name, budget in SHAPE_BUDGETS.items():
        body = [line for line in bodies.get(name, []) if INSTRUCTION.match(line)]
        if not body:
            problems.append(f"shapes: {name} is missing from the listing")
            continue
        text = "\n".join(body)
        if re.search(r"\bpush", text):
            problems.append(f"shapes: {name} saves a register:\n{text}")
        elif "(%rsp)" in text and name not in ("read_through", "exchange"):
            problems.append(f"shapes: {name} touches the stack:\n{text}")
        elif len(body) > budget:
            problems.append(
                f"shapes: {name} is {len(body)} instructions, budget {budget}:\n{text}"
            )
    return problems


def can_run(compiler, arch, env, workdir):
    probe = pathlib.Path(workdir) / f"probe{arch}.c"
    probe.write_text("int main(void) { return 0; }\n")
    exe = pathlib.Path(workdir) / f"probe{arch}"
    if run([compiler, arch, "-O1", "-o", str(exe), str(probe)], env).returncode != 0:
        return False
    try:
        return run([str(exe)], env, timeout=20).returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--lccc", default=str(ROOT / "target" / "release" / "lccc"))
    parser.add_argument("--gcc", help="also run the fixture under this GCC")
    parser.add_argument("--pairs", type=int, default=96)
    parser.add_argument("--seed", type=int, default=20261119)
    parser.add_argument("--only", help="comma list of ARCH:OPT rows, e.g. -m64:-O2")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--keep", help="write the fixture into this directory")
    args = parser.parse_args()

    if not os.access(args.lccc, os.X_OK):
        print(f"FAIL: {args.lccc} is not an executable compiler")
        return 1
    env = dict(os.environ)
    rows = [(arch, opt, ()) for arch in ("-m64", "-m32") for opt in ("-O0", "-O1", "-O2", "-O3", "-Os")]
    rows.append(("-m64", "-O2", ("-mfunction-return=thunk-extern",)))
    if args.only:
        wanted = {tuple(item.split(":")) for item in args.only.split(",")}
        rows = [row for row in rows if (row[0], row[1]) in wanted]

    problems, notes, ran = [], [], 0
    with tempfile.TemporaryDirectory(prefix="asm-operand-") as workdir:
        keep = pathlib.Path(args.keep) if args.keep else pathlib.Path(workdir)
        keep.mkdir(parents=True, exist_ok=True)
        source = keep / "asm_operand_equivalence.c"
        source.write_text(generate(args.pairs, args.seed))
        runnable = {arch: can_run(args.lccc, arch, env, workdir) for arch in {r[0] for r in rows}}
        for arch, opt, extra in rows:
            label = " ".join((arch, opt) + extra)
            if not runnable[arch]:
                if f"{arch}: skipped" not in " ".join(notes):
                    notes.append(f"{arch}: skipped, the host cannot build or run it")
                continue
            exe = pathlib.Path(workdir) / f"lccc{len(problems)}{ran}"
            status, tail = build_and_run(args.lccc, source, exe, [arch, opt, *extra], env)
            ran += 1
            if status != 0:
                problems.append(f"{label}: {tail if status is None else tail[0]}")
            # GCC vouches for the fixture (asm and C halves agree); once per
            # target is enough for that.
            if args.gcc and not extra and opt == "-O2":
                status, tail = build_and_run(
                    args.gcc, source, pathlib.Path(workdir) / "gcc", [arch, opt], env
                )
                if status not in (0, None):
                    problems.append(f"{label}: the fixture disagrees with itself under GCC: {tail[0]}")
        if runnable.get("-m64"):
            problems += check_shapes(args.lccc, workdir, env)
            problems += check_shapes(args.lccc, workdir, env, ("-mfunction-return=thunk-extern",))
        if args.self_test:
            # A fixture whose reference half is wrong must be reported: the
            # first reference function starts its checksum one off.
            original = generate(8, args.seed)
            head, sep, tail_text = original.partition("uword ref_0(")
            planted = head + sep + tail_text.replace("0x1234567;", "0x1234568;", 1)
            bad = pathlib.Path(workdir) / "planted.c"
            bad.write_text(planted)
            status, tail = build_and_run(
                args.lccc, bad, pathlib.Path(workdir) / "planted", ["-m64", "-O2"], env
            )
            if planted == original:
                problems.append("self-test: the planted fixture is the original")
            elif status != 1:
                problems.append(f"self-test: a wrong reference went unnoticed ({status}, {tail})")
            # A budget that cannot be met must be reported.
            saved = dict(SHAPE_BUDGETS)
            SHAPE_BUDGETS["inc_through"] = 1
            if not check_shapes(args.lccc, workdir, env):
                problems.append("self-test: an exceeded budget went unnoticed")
            SHAPE_BUDGETS.update(saved)

    for note in notes:
        print(f"note: {note}")
    if problems:
        for problem in problems:
            print(f"FAIL: {problem}")
        print(f"FAIL: {len(problems)} problem(s)")
        return 1
    print(
        f"PASS: asm operand equivalence: {args.pairs} pairs agree in {ran} rows; "
        f"{len(SHAPE_BUDGETS)} shapes within their budgets with and without the return thunk"
        + ("; self-test ok" if args.self_test else "")
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
