#!/usr/bin/env python3
"""Static audit of x86 frame addressing in emitted assembly.

`tests/regression/huge_frame_*` prove that lccc *compiles* frames past 2 GiB and
`huge_frame_runtime.c` proves that the addresses it picks are right when they
execute -- but only for the shapes that case happens to contain. Two whole
classes of defect are better caught by reading the text, because they are
properties of every operand rather than of one program:

  * **Encoding.** x86 memory operands carry a signed 32-bit displacement, in
    both the plain and the SIB form. A displacement outside `[-2**31,
    2**31-1]` is not a slow instruction, it is an assembler error -- and it
    reaches the assembler only after codegen has already committed to a
    layout. `huge_frame_runtime.c` found one such fold by refusing to
    assemble at -O1 (`-3221225544(%rbp, %r11)`); this audit makes that a
    property, not a coincidence.

  * **Anchor.** A `%rbp`-relative operand in a function that never established
    `%rbp` is a wrong address, not a slow one -- the register holds a caller's
    value. Conversely an `%rsp`-relative operand in a frame-pointer function
    whose frame is larger than `imm32` must be spelled against the frame size
    (the `%rsp` anchor), so its magnitude is bounded by the frame.

The audit tracks `%rsp` through the text, per function: `push`, `pop`,
`sub`/`add` of an immediate, `leave`, and the `# LCCC_STACK_ADJUST` marker
`emit_stack_adjust_x86` writes (the only way a dynamic `subq %r11, %rsp`
probe loop states its net effect). It then checks, for every memory operand:

  1. the displacement fits in signed 32 bits;
  2. `%rbp` is a legal base: the function established it;
  3. `%rsp`-relative addresses stay inside the frame (|address| <= frame size),
     which is what a wrong anchor violates.

Usage::

    python3 scripts/giant_frame_asm_audit.py                    # the corpus
    python3 scripts/giant_frame_asm_audit.py --file foo.c --levels 0,2,3
    python3 scripts/giant_frame_asm_audit.py --keep /tmp/audit  # keep the .s

Exit status is 0 when every function at every level passes, 1 otherwise.
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
LEVELS = ('0', '1', '2', 's')

# A memory operand: `disp(base)`, `disp(base, index)`, `disp(base, index, scale)`
# or the index-only form `sym(, index)`. Registers are `%`-prefixed except in
# the `(base, index)` slot positions, which the text always writes bare.
MEM_OPERAND = re.compile(
    r'(?P<disp>[-+]?(?:0x[0-9a-fA-F]+|\d+))?'
    r'\(\s*(?P<base>%?[a-z0-9]+)?\s*'
    r'(?:,\s*(?P<index>%?[a-z0-9]+)\s*(?:,\s*(?P<scale>[1248]))?\s*)?\)'
)

IMM32_MIN = -(2 ** 31)
IMM32_MAX = 2 ** 31 - 1

# Lines whose first operand is a memory operand but which are not accesses
# (they name a symbol or a jump target); none of them can appear with a
# frame-relative displacement, so a match on them is a parse artifact.
NON_ACCESS_PREFIXES = ('.', '#', '    .', 'j', 'call', 'ret')

STACK_ADJUST = re.compile(r'#\s*LCCC_STACK_ADJUST\s+(-?\d+)')


class Function:
    """One emitted function: its name, its lines and its frame facts.

    The facts are computed by `finalize()` once the whole body has been
    collected -- a function's frame size is stated by its prologue, but the
    parser cannot know where the prologue ends until the body is complete."""

    def __init__(self, name: str, lines: list[str]):
        self.name = name
        self.lines = lines
        self.has_frame_pointer = False
        self.frame_size = 0

    def finalize(self) -> None:
        # `movq %rsp, %rbp` is how this backend establishes the frame register,
        # in both the plain and the push/pop prologue forms. A function that
        # never issues it has no `%rbp` to anchor a slot on.
        self.has_frame_pointer = any('movq %rsp, %rbp' in line for line in self.lines)
        for line in self.lines:
            match = STACK_ADJUST.search(line)
            if match:
                self.frame_size = max(self.frame_size, abs(int(match.group(1))))
                continue
            sub = re.match(r'subq\s+\$(-?\d+),\s*%rsp', line.strip())
            if sub:
                self.frame_size = max(self.frame_size, int(sub.group(1)))

    def addressable_frame(self) -> int:
        """Bytes of frame a slot can sit in, for the magnitude check."""
        return self.frame_size


def parse_functions(text: str) -> list[Function]:
    """Split emitted assembly into functions by `.type <name>, @function`.

    Everything between the `.type` directive and the matching `.size` belongs to
    the function, including the CFI directives and the label, which is what the
    prologue scan needs."""
    functions: list[Function] = []
    current: Function | None = None
    for line in text.splitlines():
        marker = re.match(r'\.type\s+([A-Za-z_.$][\w.$]*)\s*,\s*@function', line)
        if marker:
            current = Function(marker.group(1), [])
            functions.append(current)
            continue
        if current is None:
            continue
        if re.match(r'\.size\s+', line):
            current.finalize()
            current = None
            continue
        current.lines.append(line)
    if current is not None:
        current.finalize()
    return functions


def _walk_rsp(lines: list[str]):
    """Yield `(line, rsp_before)` with `rsp` the live displacement below entry.

    `%rsp` moves through `push`/`pop`, an immediate `sub`/`add`, `leave`, and the
    `# LCCC_STACK_ADJUST` marker that the prologue writes for a dynamic
    adjustment (the probe loop's net effect)."""
    rsp = 0
    for line in lines:
        yield line, rsp
        stripped = line.strip()
        adjust = STACK_ADJUST.search(line)
        if adjust:
            rsp += int(adjust.group(1))
            continue
        if stripped.startswith('push'):
            rsp += 8
            continue
        if stripped.startswith('pop'):
            rsp -= 8
            continue
        if stripped.startswith('leave'):
            rsp = 0
            continue
        match = re.match(r'(subq|addq)\s+\$(-?\d+),\s*%rsp', stripped)
        if match:
            delta = int(match.group(2))
            rsp += delta if match.group(1) == 'subq' else -delta


def audit_function(func: Function, level: str) -> list[str]:
    """Return one message per violation (empty when the function is clean)."""
    problems: list[str] = []
    # The frame extends from entry %rsp down to the deepest %rsp the function
    # ever establishes; an `%rsp`-relative access must land inside that extent.
    extent = max((rsp for _, rsp in _walk_rsp(func.lines)), default=0)
    for line, rsp in _walk_rsp(func.lines):
        stripped = line.strip()
        if stripped.startswith('.') or stripped.startswith('#'):
            continue
        for operand in MEM_OPERAND.finditer(_operands_only(stripped)):
            base = operand.group('base') or ''
            if base not in ('%rbp', '%rsp'):
                continue
            disp_text = operand.group('disp')
            disp = int(disp_text, 0) if disp_text else 0
            if not IMM32_MIN <= disp <= IMM32_MAX:
                problems.append(
                    f'-O{level} {func.name}: displacement {disp} of '
                    f'`{operand.group(0)}` does not fit signed 32 bits '
                    f'(line: {stripped!r})'
                )
            if base == '%rbp':
                if not func.has_frame_pointer:
                    problems.append(
                        f'-O{level} {func.name}: `%rbp`-relative operand in a '
                        f'frame-pointer-less function (line: {stripped!r})'
                    )
                continue
            # `%rsp + disp` names an address `disp` bytes above the live %rsp,
            # i.e. `rsp - disp` below the entry %rsp. A negative depth is above
            # the entry %rsp, which is the caller's frame (incoming stack
            # arguments) and legal; only an access that lands *below* the frame
            # this function allocated can be a wrong anchor.
            depth = rsp - disp
            if depth > extent + 8:
                problems.append(
                    f'-O{level} {func.name}: `%rsp`-relative access at depth '
                    f'{depth} is outside the {extent}-byte frame '
                    f'(line: {stripped!r})'
                )
    return problems


def _operands_only(line: str) -> str:
    """Drop the mnemonic so `imulq $1216, %r11, %r11` is not read as a memory
    operand and `callq 0x10(%rip)` is still scanned."""
    space = line.find(' ')
    return line[space + 1:] if space >= 0 else ''


def audit_source(lccc: Path, source: Path, level: str, scratch: Path) -> tuple[list[str], str]:
    output = scratch / f'{source.stem}-O{level}.s'
    extra = source.with_suffix('.flags')
    flags: list[str] = []
    if extra.exists():
        flags = [f for f in extra.read_text().split() if f not in ('-O0', '-O1', '-O2', '-O3', '-Os')]
    built = subprocess.run(
        [str(lccc), '-std=gnu23', f'-O{level}', '-S', '-I', str(source.parent), *flags,
         str(source), '-o', str(output)],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=False,
    )
    if built.returncode != 0:
        return ([f'-O{level} {source.name}: lccc refused to emit assembly: '
                 f'{built.stderr.strip()[:400]}'], '')
    text = output.read_text()
    problems: list[str] = []
    functions = parse_functions(text)
    if not functions:
        problems.append(f'-O{level} {source.name}: no function parsed out of the assembly')
    for func in functions:
        problems.extend(audit_function(func, level))
    return problems, text


SELF_TEST_SAMPLES = {
    'encoding': """
.type bad_disp, @function
bad_disp:
    .cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    movb %al, -3221225544(%rbp, %r11)
    ret
    .cfi_endproc
.size bad_disp, .-bad_disp
""",
    'anchor': """
.type bad_anchor, @function
bad_anchor:
    .cfi_startproc
    pushq %rbx
    movq %rax, -16(%rbp)
    ret
    .cfi_endproc
.size bad_anchor, .-bad_anchor
""",
    'extent': """
.type bad_extent, @function
bad_extent:
    .cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    pushq %rbx
    # LCCC_STACK_ADJUST 4096
    movq $4096, %r11
    subq %r11, %rsp
    leaq 0(%rsp), %rcx
    movq -8192(%rsp), %rax
    ret
    .cfi_endproc
.size bad_extent, .-bad_extent
""",
    'clean': """
.type clean_fn, @function
clean_fn:
    .cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    pushq %rbx
    # LCCC_STACK_ADJUST 3221225472
    movq $3221225472, %r11
    subq %r11, %rsp
    movb %al, 0(%rsp)
    movb %al, 1073741824(%rsp)
    movq %rax, -16(%rbp)
    leaq -8(%rbp), %rsp
    popq %rbp
    ret
    .cfi_endproc
.size clean_fn, .-clean_fn
""",
}


def self_test() -> int:
    """Prove the checker rejects each violation class it claims to catch.

    A verifier that only ever prints "clean" is worthless: each sample below
    carries exactly one defect (or none) and the expected verdict is asserted,
    so a refactor that silently stops matching fails here instead of passing
    every audit."""
    expected = {'encoding': False, 'anchor': False, 'extent': False, 'clean': True}
    failures = 0
    for name, text in SELF_TEST_SAMPLES.items():
        functions = parse_functions(text)
        if len(functions) != 1:
            print(f'self-test {name}: parsed {len(functions)} functions, expected 1')
            failures += 1
            continue
        problems = audit_function(functions[0], 'T')
        passed = not problems
        if passed != expected[name]:
            print(f'self-test {name}: expected pass={expected[name]}, got pass={passed}: {problems}')
            failures += 1
            continue
        print(f'  self-test {name}: {"clean" if passed else problems[0]}')
    return failures


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--lccc', default=str(ROOT / 'target/fastbuild/lccc'))
    parser.add_argument('--file', action='append', default=None,
                        help='source to audit (default: the giant-frame corpus)')
    parser.add_argument('--levels', default=','.join(LEVELS))
    parser.add_argument('--keep', default=None, help='directory to keep the .s files in')
    parser.add_argument('--verbose', '-v', action='store_true')
    parser.add_argument('--self-test', action='store_true',
                        help='check the checker against known-bad samples and exit')
    args = parser.parse_args(argv)

    if args.self_test:
        failures = self_test()
        print('frame addressing self-test: '
              + ('ok' if failures == 0 else f'{failures} failure(s)'))
        return 0 if failures == 0 else 1

    lccc = Path(args.lccc).resolve()
    if not lccc.exists():
        print(f'error: {lccc} does not exist; build it first', file=sys.stderr)
        return 2

    sources = [Path(f).resolve() for f in args.file] if args.file else sorted(
        (ROOT / 'tests/regression').glob('huge_frame*.c'))
    if not sources:
        print('error: no sources to audit', file=sys.stderr)
        return 2
    levels = [level.strip() for level in args.levels.split(',') if level.strip()]

    scratch = Path(args.keep).resolve() if args.keep else Path(tempfile.mkdtemp(prefix='lccc-frame-audit-'))
    scratch.mkdir(parents=True, exist_ok=True)
    problems: list[str] = []
    operands = 0
    try:
        for source in sources:
            for level in levels:
                found, text = audit_source(lccc, source, level, scratch)
                problems.extend(found)
                operands += len(re.findall(r'\(%r(?:bp|sp)', text))
                if args.verbose and not found:
                    print(f'  {source.name} -O{level}: clean')
    finally:
        if args.keep is None:
            shutil.rmtree(scratch, ignore_errors=True)

    if problems:
        print(f'FAILED: {len(problems)} addressing violation(s)\n')
        for problem in problems:
            print(f'  {problem}')
        return 1
    print(f'frame addressing: {len(sources)} source(s) x {len(levels)} level(s) clean '
          f'({operands} frame-relative operands checked)')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
