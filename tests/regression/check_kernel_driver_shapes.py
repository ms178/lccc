#!/usr/bin/env python3
"""What building amdgpu, ixgbe and KVM asked of the compiler, held as a gate.

Linux 6.18.63 with the CachyMod configuration (drivers/gpu/drm/amd,
drivers/net/ethernet/intel/ixgbe, arch/x86/kvm, the core kernel) found seven
things in LCCC that a kernel built from defconfig had not:

* an inline-only wrapper around a library function (`fortify-string.h`:
  `extern __always_inline __gnu_inline memcmp(...)` returning
  `__builtin_memcmp(...)`) was inlined into itself and looped;
* a parameter declared as an array of pointers to function lost its array
  (`int (*handlers[])(struct kvm_vcpu *)`), so KVM did not compile;
* `case X: break;` left a `jmp` behind the jump table, and a `default:` no
  value selects stayed in the function -- both `unreachable instruction` to
  objtool;
* `goto *p` with `p` always `&&retry` (`drm_exec_until_all_locked`) stayed an
  indirect jump, which objtool reads as a tail call out of a live frame;
* a by-value aggregate of one register (`struct bw_fixed`, `pte_t`) got a
  stack image per call: `calculate_bandwidth` had a 7832-byte frame, GCC 240;
* GNU `unused` on a struct type did not reach its variables, so each one in a
  header was reported (`-Wunused-const-variable`);
* one pass never returned on `dce_clk_mgr.c`.

Its assembly sources added three more: touching macro references of unequal
length (`KEY\\i\\xmm_suffix` in aes-xts-avx-x86_64.S), the `pclmulhqlqdq`
family of aliases, and the VMX instructions.

Each has unit tests next to its code.  This gate holds the compiled result:

  run     the six fixtures in tests/regression compute what GCC computes,
          -O0..-Os, x86-64 and i686;
  reach   every instruction LCCC emits for them can be reached (x86-64,
          -O0..-Os) -- objtool's `unreachable instruction`, on the text;
  shape   no indirect jump for the one-label goto; the frame of a chain of
          by-value calls does not grow with the chain; the wrapper reaches
          the library once and keeps its check;
  abi     a caller built by LCCC and a callee built by GCC (and the reverse)
          agree on one-register aggregates in every argument position;
  diag    the unused warnings and the spelling of declarator types are GCC's;
  asm     what the kernel's .S files asked of the assembler: every source in
          the list assembles to the bytes and the relocations GNU as
          produces, or is refused by both (macro references, repeat blocks,
          the carry-less multiply aliases, VMX, nested ALTERNATIVEs, who
          binds a branch, references into mergeable sections); and a weak
          definition reached by `jmp`, `jcc` and `call` is still replaced
          at link time.  NOP padding is not compared here: its byte pattern
          depends on the GNU as release, and the unit tests pin it to 2.47's.

    check_kernel_driver_shapes.py --lccc PATH [--gcc gcc-16]
    check_kernel_driver_shapes.py --lccc PATH --self-test   the checks see planted faults

No third-party packages.  Exit status 0 = clean.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIXTURES = [
    "param_array_of_function_pointers",
    "fortify_gnu_inline_wrapper",
    "switch_break_cases_covered_default",
    "switch_cases_cover_selector",
    "computed_goto_one_label",
    "byval_one_register_aggregate",
]
LEVELS = ["-O0", "-O1", "-O2", "-O3", "-Os"]
NORETURN = {"abort", "exit", "_exit", "__stack_chk_fail", "__assert_fail", "__builtin_trap"}


class Failure(Exception):
    pass


def run(cmd, *, env=None, timeout=120, cwd=None):
    full_env = dict(os.environ)
    if env:
        full_env.update(env)
    return subprocess.run(
        [str(c) for c in cmd], capture_output=True, text=True, timeout=timeout, env=full_env, cwd=cwd
    )


# --------------------------------------------------------------------------
# reach: every emitted instruction has a way in
# --------------------------------------------------------------------------

LABEL = re.compile(r"^([.\w$]+):\s*(.*)$")
LOCAL_REF = re.compile(r"(?<![\w.$])(\.L[\w.$]+)")
JCC = re.compile(r"^(j[a-z]+|loop[a-z]*|jr?cxz)$")


def functions_of(asm: str):
    """Yield (name, lines) for each function body in LCCC's assembly text."""
    lines = asm.split("\n")
    names = re.findall(r"^\s*\.type\s+([\w.$]+),\s*@function", asm, flags=re.M)
    for name in names:
        try:
            start = next(i for i, line in enumerate(lines) if line.strip() == f"{name}:")
        except StopIteration:
            continue
        end = next(
            (i for i in range(start, len(lines)) if re.match(rf"\s*\.size\s+{re.escape(name)}\s*,", lines[i])),
            len(lines),
        )
        yield name, lines[start + 1 : end]


def data_label_refs(asm: str):
    """Local labels named by a data directive anywhere in the file: the
    table of a `static void *t[] = { &&a, &&b }` lives outside its function."""
    refs = set()
    for line in asm.split("\n"):
        stripped = line.strip()
        if re.match(r"\.(quad|long|8byte|4byte)\b", stripped):
            refs.update(LOCAL_REF.findall(stripped))
    return refs


def unreachable_instructions(body, roots=()):
    """Instructions of one function body that no path from its entry reaches.

    The text is read the way the assembler reads it: only what lands in the
    function's own section is code, a label named by data (a jump table, a
    table of `&&label`) is a way in, and `jmp`, `ret`, `ud2`, an indirect jump
    and a call that does not return do not fall through.
    """
    code = []  # (kind, text, targets, falls_through)
    index_of_label = {}
    data_refs = set(roots)
    stack, current, previous = [], "text", "text"
    in_asm = False
    for raw in body:
        line = raw.split("#", 1)[0].rstrip() if not raw.lstrip().startswith("#") else raw.strip()
        stripped = line.strip()
        if stripped == "#APP":
            in_asm = True
            continue
        if stripped == "#NO_APP":
            in_asm = False
            continue
        if not stripped or stripped.startswith("#"):
            continue
        for stmt in [s.strip() for s in stripped.split(";")] if in_asm else [stripped]:
            if not stmt:
                continue
            m = LABEL.match(stmt)
            while m:
                if current == "text":
                    index_of_label[m.group(1)] = len(code)
                stmt = m.group(2).strip()
                m = LABEL.match(stmt) if stmt else None
            if not stmt:
                continue
            word = stmt.split()[0]
            if word.startswith("."):
                directive = word[1:].lower()
                if directive == "pushsection":
                    stack.append((current, previous))
                    previous, current = current, "other"
                elif directive == "popsection":
                    if stack:
                        current, previous = stack.pop()
                elif directive in ("section", "data", "bss", "text"):
                    name = stmt.split(None, 1)[1].split(",")[0].strip() if directive == "section" else "." + directive
                    is_text = name.startswith(".text") or name in (".init.text",)
                    previous, current = current, ("text" if is_text else "other")
                elif directive == "previous":
                    current, previous = previous, current
                elif current != "text" or directive in ("quad", "long"):
                    data_refs.update(LOCAL_REF.findall(stmt))
                continue
            if current != "text":
                continue
            operands = stmt[len(word) :].strip()
            targets = LOCAL_REF.findall(operands)
            mnemonic = word.lower()
            if in_asm:
                code.append(("asm", stmt, targets, True))
            elif mnemonic in ("jmp", "jmpq"):
                indirect = operands.startswith("*")
                code.append(("jmp", stmt, [] if indirect else targets, False))
            elif mnemonic in ("ret", "retq", "ud2", "hlt"):
                code.append(("end", stmt, [], False))
            elif JCC.match(mnemonic):
                code.append(("jcc", stmt, targets, True))
            elif mnemonic in ("call", "callq"):
                callee = operands.split("@")[0]
                code.append(("call", stmt, [], callee not in NORETURN))
            else:
                # An address taken into a register (`leaq .LBB1(%rip)`) is a
                # way in as well: someone may jump through it.
                data_refs.update(t for t in targets if mnemonic.startswith("lea") or mnemonic.startswith("mov"))
                code.append(("insn", stmt, [], True))
    seen = [False] * len(code)
    work = [0] if code else []
    work += [index_of_label[label] for label in data_refs if label in index_of_label]
    while work:
        i = work.pop()
        while i < len(code) and not seen[i]:
            seen[i] = True
            _, _, targets, falls = code[i]
            for target in targets:
                j = index_of_label.get(target)
                if j is not None and j < len(code) and not seen[j]:
                    work.append(j)
            if not falls:
                break
            i += 1
    return [code[i][1] for i in range(len(code)) if not seen[i]]


def check_reach(lccc: Path, tmp: Path, env=None):
    count = 0
    for name in FIXTURES:
        for level in LEVELS:
            asm = tmp / f"{name}{level}.s"
            p = run([lccc, level, "-S", HERE / f"{name}.c", "-o", asm], env=env)
            if p.returncode != 0:
                raise Failure(f"reach: {name} {level} does not compile:\n{p.stderr}")
            text = asm.read_text()
            roots = data_label_refs(text)
            for func, body in functions_of(text):
                dead = unreachable_instructions(body, roots)
                if dead:
                    raise Failure(
                        f"reach: {name}.c {level}: {func} has {len(dead)} instruction(s) nothing reaches "
                        f"(objtool: unreachable instruction):\n    " + "\n    ".join(dead[:6])
                    )
                count += 1
    return count


# --------------------------------------------------------------------------
# shape
# --------------------------------------------------------------------------


def function_body(asm: str, name: str):
    for func, body in functions_of(asm):
        if func == name:
            return [line.split("#", 1)[0].strip() for line in body if line.strip() and not line.strip().startswith("#")]
    raise Failure(f"shape: no function {name} in the assembly")


def chain_source(statements: int) -> str:
    """`calculate_bandwidth` in small: each temporary feeds the next call."""
    lines = [
        "struct bw_fixed { long long value; };",
        "struct bw_fixed bw_mul(struct bw_fixed, struct bw_fixed);",
        "struct bw_fixed bw_div(struct bw_fixed, struct bw_fixed);",
        "struct bw_fixed bw_int_to_fixed(long long);",
        "static inline struct bw_fixed bw_add(struct bw_fixed a, struct bw_fixed b)",
        "{ struct bw_fixed r; r.value = a.value + b.value; return r; }",
        "struct data { struct bw_fixed in[64]; struct bw_fixed out[64]; int n[64]; };",
        "void chain(struct data *d) {",
    ]
    for k in range(statements):
        lines.append(
            f"    d->out[{k}] = bw_mul(bw_add(bw_mul(d->in[{k}], d->in[{k + 1}]), "
            f"bw_div(d->in[{k + 2}], d->in[{k + 3}])), bw_int_to_fixed(d->n[{k}]));"
        )
    lines.append("}")
    return "\n".join(lines) + "\n"


def frame_bytes(body) -> int:
    total = 0
    for line in body:
        m = re.match(r"subq\s+\$(\d+),\s*%rsp", line)
        if m:
            total = max(total, int(m.group(1)))
    return total


def check_shape(lccc: Path, tmp: Path, env=None):
    count = 0
    # One label behind the pointer: a direct jump, on both x86 targets.
    for suffix in ("", "-i686"):
        for level in LEVELS[1:]:
            asm = tmp / f"goto{suffix}{level}.s"
            p = run([f"{lccc}{suffix}", level, "-S", HERE / "computed_goto_one_label.c", "-o", asm], env=env)
            if p.returncode != 0:
                raise Failure(f"shape: computed_goto_one_label{suffix} {level}:\n{p.stderr}")
            text = asm.read_text()
            body = function_body(text, "lock_all")
            indirect = [line for line in body if re.match(r"jmpq?\s+\*", line)]
            if indirect:
                raise Failure(
                    f"shape: lock_all{suffix} {level} jumps through a register for its one label "
                    f"(GCC and Clang emit a direct jmp):\n    " + "\n    ".join(indirect)
                )
            # The dispatch through a table keeps its indirect jump.
            if not any(re.match(r"jmpq?\s+\*", line) for line in function_body(text, "interpret")):
                raise Failure(f"shape: interpret{suffix} {level} lost its table dispatch")
            count += 2
    # The frame of a chain of by-value calls does not grow with the chain.
    frames = {}
    for statements in (4, 24):
        src = tmp / f"chain{statements}.c"
        src.write_text(chain_source(statements))
        for level in ("-O2", "-O3", "-Os"):
            asm = tmp / f"chain{statements}{level}.s"
            p = run([lccc, level, "-S", src, "-o", asm], env=env)
            if p.returncode != 0:
                raise Failure(f"shape: chain{statements} {level}:\n{p.stderr}")
            frames[(statements, level)] = frame_bytes(function_body(asm.read_text(), "chain"))
    for level in ("-O2", "-O3", "-Os"):
        small, large = frames[(4, level)], frames[(24, level)]
        if large > small + 16 or large > 64:
            raise Failure(
                f"shape: the frame of a chain of by-value calls grows with the chain at {level}: "
                f"{small} bytes for 4 statements, {large} for 24 (GCC 16.2: 0 and 0)"
            )
        count += 1
    # The fortified wrapper: the library once per call, the check kept.
    for suffix in ("", "-i686"):
        for level in LEVELS:
            asm = tmp / f"fortify{suffix}{level}.s"
            p = run([f"{lccc}{suffix}", level, "-S", HERE / "fortify_gnu_inline_wrapper.c", "-o", asm], env=env)
            if p.returncode != 0:
                raise Failure(f"shape: fortify{suffix} {level}:\n{p.stderr}")
            text = asm.read_text()
            body = function_body(text, "patch_site")
            calls = [line for line in body if re.match(r"call\s+memc(mp|py)\b", line)]
            if not any(line in ("ret", "retq") or line.startswith("jmp __x86_return_thunk") for line in body):
                raise Failure(f"shape: patch_site{suffix} {level} has no return")
            if len(calls) > 2:
                raise Failure(
                    f"shape: patch_site{suffix} {level} calls the library {len(calls)} times for one "
                    f"memcmp and one memcpy (the wrapper was inlined into itself)"
                )
            checked = function_body(text, "checked")
            if not any(re.match(r"call\s+fortify_panic\b", line) for line in checked):
                raise Failure(f"shape: checked{suffix} {level} lost the wrapper's check")
            if re.search(r"^\s*\.type\s+(memcmp|memcpy|memset|strlen|strcpy),\s*@function", text, re.M):
                raise Failure(f"shape: fortify{suffix} {level} defines a library function out of line")
            count += 1
    return count


# --------------------------------------------------------------------------
# run: the fixtures compute what GCC computes
# --------------------------------------------------------------------------


def can_run_m32(lccc: Path, tmp: Path) -> bool:
    src = tmp / "m32probe.c"
    src.write_text("int main(void) { return 0; }\n")
    exe = tmp / "m32probe"
    p = run([f"{lccc}-i686", "-O1", src, "-o", exe])
    if p.returncode != 0:
        return False
    try:
        return run([exe]).returncode == 0
    except OSError:
        return False


def check_run(lccc: Path, gcc: str, tmp: Path, m32: bool):
    count = 0
    for name in FIXTURES:
        src = HERE / f"{name}.c"
        ref = tmp / f"{name}.gcc"
        p = run([gcc, "-O2", "-w", src, "-o", ref])
        if p.returncode != 0:
            raise Failure(f"run: {gcc} does not compile {name}.c:\n{p.stderr}")
        want = run([ref])
        if want.returncode != 0:
            raise Failure(f"run: the {gcc} build of {name}.c fails its own checks")
        for suffix in ("", "-i686") if m32 else ("",):
            for level in LEVELS:
                exe = tmp / f"{name}{suffix}{level}"
                p = run([f"{lccc}{suffix}", level, src, "-o", exe])
                if p.returncode != 0:
                    raise Failure(f"run: {name}{suffix} {level} does not compile:\n{p.stderr}")
                got = run([exe], timeout=30)
                if got.returncode != want.returncode or got.stdout != want.stdout:
                    raise Failure(
                        f"run: {name}{suffix} {level} differs from {gcc}:\n--- {gcc}\n{want.stdout}"
                        f"--- lccc (exit {got.returncode})\n{got.stdout}"
                    )
                count += 1
    return count


# --------------------------------------------------------------------------
# abi: one-register aggregates across two compilers
# --------------------------------------------------------------------------

ABI_HEADER = r"""
struct fixed { long long value; };
struct word { int v; };
struct byte { unsigned char v; };
struct ptr { const char *p; };
struct pair { long a, b; };
struct fixed fx(long long v);
struct fixed mul(struct fixed a, struct fixed b);
long long spread(struct fixed a, long b, struct fixed c, int d, struct fixed e, struct fixed f,
                 struct fixed g, long h, struct fixed i, struct fixed j);
int narrow(struct byte a, struct word b, struct ptr c, struct byte d);
long long variadic(int n, ...);
long with_pair(struct fixed a, struct pair p, struct fixed b);
double with_double(double x, struct fixed a, double y, struct fixed b);
"""

ABI_CALLEE = (
    ABI_HEADER
    + r"""
#include <stdarg.h>
struct fixed fx(long long v) { struct fixed r = { v }; return r; }
struct fixed mul(struct fixed a, struct fixed b) { return fx(a.value * b.value); }
long long spread(struct fixed a, long b, struct fixed c, int d, struct fixed e, struct fixed f,
                 struct fixed g, long h, struct fixed i, struct fixed j)
{
    return a.value + 2 * b + 3 * c.value + 4 * d + 5 * e.value + 6 * f.value + 7 * g.value +
           8 * h + 9 * i.value + 10 * j.value;
}
int narrow(struct byte a, struct word b, struct ptr c, struct byte d)
{
    return a.v + b.v * 3 + c.p[1] * 5 + d.v * 7;
}
long long variadic(int n, ...)
{
    va_list ap;
    long long s = 0;
    va_start(ap, n);
    for (int k = 0; k < n; k++)
        s = s * 3 + va_arg(ap, struct fixed).value;
    va_end(ap);
    return s;
}
long with_pair(struct fixed a, struct pair p, struct fixed b)
{
    return a.value * 1000 + p.a * 100 + p.b * 10 + b.value;
}
double with_double(double x, struct fixed a, double y, struct fixed b)
{
    return x * a.value + y * b.value;
}
"""
)

ABI_CALLER = (
    ABI_HEADER
    + r"""
extern int printf(const char *, ...);
static inline struct fixed add(struct fixed a, struct fixed b) { return fx(a.value + b.value); }
int main(void)
{
    struct pair p = { 4, 5 };
    struct fixed in[4] = { { 11 }, { -7 }, { 5 }, { 3 } };
    /* Every argument is a value that was just computed: nothing to read it from. */
    printf("mul %lld\n", mul(add(in[0], in[1]), add(in[2], in[3])).value);
    printf("spread %lld\n", spread(add(in[0], in[1]), 3, mul(in[2], in[3]), 5, add(in[1], in[2]),
                                   mul(in[0], in[3]), add(in[3], in[3]), 9, mul(in[1], in[1]),
                                   add(in[0], in[2])));
    printf("narrow %d\n", narrow((struct byte){ (unsigned char)(in[0].value * 20) },
                                 (struct word){ (int)(in[1].value * 10001) }, (struct ptr){ "xyz" },
                                 (struct byte){ (unsigned char)in[2].value }));
    printf("variadic %lld\n", variadic(4, add(in[0], in[1]), mul(in[2], in[3]), in[3],
                                       add(in[1], in[1])));
    printf("pair %ld\n", with_pair(add(in[0], in[1]), p, mul(in[2], in[3])));
    printf("double %.3f\n", with_double(1.5, add(in[0], in[1]), -2.25, mul(in[2], in[3])));
    return 0;
}
"""
)


def check_abi(lccc: Path, gcc: str, tmp: Path, env=None):
    callee, caller = tmp / "abi_callee.c", tmp / "abi_caller.c"
    callee.write_text(ABI_CALLEE)
    caller.write_text(ABI_CALLER)

    def build(cc_callee, cc_caller, level, tag, env_caller=None):
        objs = []
        for cc, src, extra_env in ((cc_callee, callee, None), (cc_caller, caller, env_caller)):
            obj = tmp / f"{src.stem}.{tag}.o"
            p = run([cc, level, "-w", "-c", src, "-o", obj], env=extra_env)
            if p.returncode != 0:
                raise Failure(f"abi: {cc} {level} {src.name}:\n{p.stderr}")
            objs.append(obj)
        exe = tmp / f"abi.{tag}"
        p = run([gcc, *objs, "-o", exe])
        if p.returncode != 0:
            raise Failure(f"abi: link {tag}:\n{p.stderr}")
        out = run([exe], timeout=30)
        return out.returncode, out.stdout

    want = build(gcc, gcc, "-O2", "ref")
    if want[0] != 0:
        raise Failure("abi: the reference build fails")
    count = 0
    for level in LEVELS:
        for tag, cc_callee, cc_caller in (
            (f"lccc-calls-gcc{level}", gcc, lccc),
            (f"gcc-calls-lccc{level}", lccc, gcc),
            (f"lccc-calls-lccc{level}", lccc, lccc),
        ):
            got = build(cc_callee, cc_caller, level, tag, env_caller=env)
            if got != want:
                raise Failure(f"abi: {tag} differs:\n--- reference\n{want[1]}--- {tag} (exit {got[0]})\n{got[1]}")
            count += 1
    return count


# --------------------------------------------------------------------------
# diag: GCC's warnings, GCC's spelling
# --------------------------------------------------------------------------

UNUSED_HEADER = r"""
struct marked { int a[2]; } __attribute__((__unused__));
struct plain { int a; };
typedef struct marked marked_t;
typedef marked_t marked_twice_t;
typedef struct { int a; } __attribute__((unused)) anonymous_t;
typedef int tagged_int __attribute__((unused));
union __attribute__((unused)) u_marked { int a; float f; };
static const struct marked HDR_MARKED = { { 1, 2 } };
static const struct plain HDR_PLAIN = { 1 };
static const marked_t HDR_TYPEDEF = { { 1 } };
static const marked_twice_t HDR_TYPEDEF2 = { { 1 } };
static const anonymous_t HDR_ANON = { 1 };
static const tagged_int HDR_INT = 1;
static const union u_marked HDR_UNION = { 1 };
static const struct marked *HDR_POINTER;
static const struct marked HDR_ARRAY[2];
static const marked_t HDR_TD_ARRAY[2];
"""
UNUSED_MAIN = r"""
#include "unused.h"
static const struct marked MAIN_MARKED = { { 1, 2 } };
static const struct plain MAIN_PLAIN = { 1 };
static struct marked main_object;
static struct plain main_plain_object;
struct member_marked { int m __attribute__((unused)); } static const MEMBER = { 1 };
int f(void)
{
    struct marked local;
    marked_t local_typedef;
    struct plain local_plain;
    struct marked *local_pointer;
    int set_only = 1;
    {
        /* A shadowing typedef without the attribute. */
        typedef struct plain marked_t;
        marked_t shadowed;
    }
    set_only = 2;
    return 0;
}
"""
UNUSED_FLAGS = [
    ["-Wall", "-Wextra"],
    ["-Wall", "-Wextra", "-Wunused-const-variable"],
    ["-Wunused-const-variable=1"],
    ["-Wunused"],
    ["-Wall", "-Wno-unused-const-variable"],
    ["-Wunused", "-Wno-unused-variable"],
]

DECLARATORS = [
    ("int (*h[])(struct S *)", "h"),
    ("int (*h[2])(struct S *)", "h"),
    ("int (*const h[])(int)", "h"),
    ("int (*h[static 2])(int)", "h"),
    ("int (**h)(struct S *)", "h"),
    ("int (***h)(int)", "h"),
    ("int *(**h)(int)", "h"),
    ("int *(*h[])(int)", "h"),
    ("int (*h[])[4]", "h"),
    ("int (**h)[4]", "h"),
    ("int (*h)(int)", "h"),
    ("int h(int)", "h"),
    ("void (*h[3])(void)", "h"),
]


def unused_signature(cc, flags, tmp: Path):
    p = run([cc, "-fsyntax-only", "-fdiagnostics-color=never", *flags, tmp / "unused.c"])
    names = re.findall(
        r"warning: (?:'(\w+)' defined but not used|unused variable '(\w+)'|"
        r"variable '(\w+)' set but not used)",
        p.stderr,
    )
    return sorted({a or b or c for a, b, c in names}), p.stderr


def quoted_type(cc, declarator: str, tmp: Path):
    src = tmp / "declarator.c"
    src.write_text(f"struct S;\nvoid f({declarator}) {{ char *c = h; (void)c; }}\n")
    p = run([cc, "-fsyntax-only", "-fdiagnostics-color=never", src])
    m = re.search(r"from incompatible pointer type '([^']+)'", p.stderr)
    return (m.group(1) if m else None), p.stderr


def check_diag(lccc: Path, gcc: str, tmp: Path):
    (tmp / "unused.h").write_text(UNUSED_HEADER)
    (tmp / "unused.c").write_text(UNUSED_MAIN)
    count = 0
    for flags in UNUSED_FLAGS:
        want, gcc_err = unused_signature(gcc, flags, tmp)
        for suffix in ("", "-i686", "-arm", "-riscv"):
            got, err = unused_signature(f"{lccc}{suffix}", flags, tmp)
            if got != want:
                raise Failure(
                    f"diag: unused warnings differ from {gcc} with {' '.join(flags)} (lccc{suffix}):\n"
                    f"  only {gcc}: {sorted(set(want) - set(got))}\n  only lccc: {sorted(set(got) - set(want))}\n"
                    f"--- lccc\n{err}--- {gcc}\n{gcc_err}"
                )
            count += 1
    for declarator, _ in DECLARATORS:
        want, gcc_err = quoted_type(gcc, declarator, tmp)
        if want is None:
            raise Failure(f"diag: {gcc} names no type for `{declarator}`:\n{gcc_err}")
        got, err = quoted_type(lccc, declarator, tmp)
        if got != want:
            raise Failure(f"diag: `{declarator}` is '{got}' to lccc and '{want}' to {gcc}\n{err}")
        count += 1
    return count


# --------------------------------------------------------------------------
# asm: the assembler against GNU as
# --------------------------------------------------------------------------

def alternative(old, new_len):
    """The kernel's ALTERNATIVE around `old`, for a replacement of `new_len` bytes."""
    return [
        "740:", *old, "741:",
        ".skip -(((744f-743f)-(741b-740b)) > 0) * ((744f-743f)-(741b-740b)),0x90",
        "742:", '.pushsection .altinstr_replacement,"ax"', "743:", f".fill {new_len},1,0xcc", "744:", ".popsection",
    ]


# Each source is assembled by both. They must agree: the same bytes in .text
# and .data, or a refusal from both. Lines are joined with newlines.
def BRANCHES_TO(name, before, after=()):
    """Every kind of direct branch to `name`, a label of their own section,
    with the directives that say what `name` is before or after them.

    Only what every GNU as since 2.32 answers alike is asked here. How a
    branch to `sym+N` that stays the linker's is marked (PC32, so that no
    PLT entry is entered N bytes in) and what a `@PLT` branch to a local
    label of another section becomes (PC32 against the section) changed
    between releases; the unit tests pin those to 2.47.
    """
    return [
        ".text", *before,
        f"jmp {name}", f"jne {name}", f"call {name}", f"jmp {name}@PLT", f"jne {name}@PLT", f"call {name}@PLT",
        f"loop {name}", f"jrcxz {name}",
        f"{name}:", "ret", *after,
    ]


ASM_CASES = {
    # -- macro references: one pass, GNU as's idea of a name ---------------
    "adjacent": [".macro m a, b", ".byte \\a\\b", ".endm", ".data", "m 1, 2"],
    "adjacent3": [".macro m a, b, c", ".byte \\a\\b\\c", ".endm", ".data", "m 1, 2, 3"],
    "longer-second": [".macro m i, suffix", ".byte 1\\i\\suffix", ".endm", ".data", "m 2, 3"],
    "prefix-names": [".macro m a, a_b", ".byte \\a_b, \\a", ".endm", ".data", "m 1, 2"],
    "dot-continues": [".macro m a", "x\\a.y = 7", ".byte x5.y", ".endm", ".data", "m 5"],
    "dot-continues-set": [".macro m a", ".set s.\\a.t, 9", ".byte s.5.t", ".endm", ".data", "m 5"],
    "dollar-continues": [".macro m a", ".set s\\a$t, 9", ".byte s5$t", ".endm", ".data", "m 5"],
    "digit-continues": [".macro m a", ".byte \\a1", ".endm", ".data", "m 2"],
    "no-rescan": [".macro m a, b", ".byte \\a", ".endm", ".data", "m \\b, 7"],
    "paren-forms": [".macro m a", ".byte \\a\\()1, \\(4)\\a", ".endm", ".data", "m 2"],
    "paren-open": [".macro m a", ".byte \\(7", ".endm", ".data", "m 1"],
    "invocation": [".macro m a", ".byte \\@, \\a", ".endm", ".data", "m 9", "m 8"],
    "invocation-arg": [".macro m a", ".byte \\a", ".endm", ".macro n", "m \\@", ".endm", ".data", "n", "n"],
    "in-string": [".macro m a", '.ascii "\\a-\\a"', ".endm", ".data", "m xy"],
    "backslashes": [".macro m a", '.ascii "\\\\a"', '.ascii "q\\\\\\a"', ".endm", ".data", "m n"],
    "unknown": [".macro m a", ".byte \\a", ".byte \\zz", ".endm", ".data", "m 1"],
    "case": [".macro m Abc", ".byte \\abc, \\ABC", ".endm", ".data", "m 5"],
    "keyword": [".macro m a=3, b", ".byte \\a\\b", ".endm", ".data", "m b=4"],
    "empty-value": [".macro m a, b", ".byte 1\\a\\b", ".endm", ".data", "m , 5"],
    "vararg": [".macro m a, b:vararg", ".byte \\a", ".byte \\b", ".endm", ".data", "m 1, 2, 3"],
    "nested-definition": [
        ".macro outer a", ".macro inner b", ".byte \\a, \\b", ".endm", ".endm", ".data", "outer 3", "inner 4",
    ],
    # -- repeat blocks ------------------------------------------------------
    "irp-adjacent": [".data", ".irp i, 1, 2", ".byte \\i\\i", ".endr"],
    "irp-dot": [".data", "x1.y = 5", "x2.y = 6", ".irp i, 1, 2", ".byte x\\i.y", ".endr"],
    "irp-invocation": [".macro m", ".byte \\@", ".endm", ".data", "m", ".irp i, 1, 2", ".byte \\@", ".endr", "m"],
    "irp-in-macro": [".macro m x", ".irp i, 1, 2", ".byte \\x\\i", ".endr", ".endm", ".data", "m 3"],
    "irpc": [".data", ".irpc c, 12", ".byte \\c\\c", ".endr"],
    "irpc-in-irp": [".data", ".irp i, 1, 2", ".irpc c, ab", ".byte \\i", ".endr", ".endr"],
    "rept-in-irp": [".data", ".irp i, 1, 2", ".rept 2", ".byte \\i", ".endr", ".endr"],
    "irp-in-rept": [".data", ".rept 2", ".irp i, 1, 2", ".byte \\i", ".endr", ".endr"],
    "three-deep": [".data", ".rept 2", ".irpc c, 56", ".rept 2", ".byte \\c", ".endr", ".endr", ".endr"],
    "unclosed-irp": [".data", ".irp i, 1", ".byte \\i"],
    "rept-symbol": [".data", ".set n, 3", ".rept n", ".byte 7", ".endr"],
    "rept-argument": [".macro m n", ".rept \\n", ".byte 5", ".endr", ".endm", ".data", "m 2"],
    # -- the round keys of aes-xts-avx-x86_64.S ----------------------------
    "aes-xts-round": [
        ".set KEY, %rdi", ".set KEY5_XMM, %xmm20", ".set KEY6_XMM, %xmm21", ".set USE_AVX512, 1",
        ".macro _vaes enc, key, data", ".if \\enc", "vaesenc \\key, \\data, \\data", ".else",
        "vaesdec \\key, \\data, \\data", ".endif", ".endm",
        ".macro _vaes_1x enc, i, xmm_suffix, data, tmp", ".if USE_AVX512",
        "_vaes \\enc, KEY\\i\\xmm_suffix, \\data", ".else", "_vaes \\enc, (\\i-7)*16(KEY), \\data", ".endif", ".endm",
        ".macro _aes_crypt enc, xmm_suffix, tweak, data, tmp", ".irp i, 5,6",
        "_vaes_1x \\enc, \\i, \\xmm_suffix, \\data, tmp=\\tmp", ".endr", ".endm",
        ".text", "_aes_crypt 1, _XMM, %xmm6, %xmm0, tmp=%xmm1", "_aes_crypt 0, _XMM, %xmm6, %xmm0, tmp=%xmm1",
    ],
    # -- carry-less multiply aliases ---------------------------------------
    "pclmul-sse": [
        ".text", "pclmullqlqdq %xmm1, %xmm2", "pclmulhqlqdq %xmm1, %xmm2", "pclmullqhqdq %xmm1, %xmm2",
        "pclmulhqhqdq (%rax), %xmm2",
    ],
    "pclmul-vex": [
        ".text", "vpclmullqlqdq %xmm1, %xmm2, %xmm3", "vpclmulhqlqdq %xmm1, %xmm2, %xmm3",
        "vpclmullqhqdq %xmm1, %xmm2, %xmm3", "vpclmulhqhqdq %xmm1, %xmm2, %xmm3",
        "vpclmullqlqdq %ymm1, %ymm2, %ymm3", "vpclmulhqlqdq (%rdi), %ymm2, %ymm3",
    ],
    "pclmul-evex": [
        ".text", "vpclmullqhqdq %zmm1, %zmm2, %zmm3", "vpclmulhqhqdq 64(%rdi), %zmm2, %zmm3",
        "vpclmulhqlqdq %xmm17, %xmm18, %xmm19", "vpclmullqlqdq %ymm20, %ymm2, %ymm30",
    ],
    "pclmul-with-immediate": [".text", "pclmullqlqdq $1, %xmm1, %xmm2"],
    # -- VMX ----------------------------------------------------------------
    "vmx": [
        ".text", "vmread %rdx, %rcx", "vmread %rax, (%rdi)", "vmwrite %rsi, %rcx", "vmwrite 16(%r13), %r10",
        "vmptrld (%rdi)", "vmptrst (%rdi)", "vmclear -8(%rbp,%r9,4)", "vmxon 24(%r8)",
        "invept (%rsi), %rdi", "invvpid (%r10), %r8",
    ],
    "vmx-wrong-direction": [".text", "vmread (%rdi), %rax"],
    # -- GFNI in its EVEX forms (aria-gfni-avx512-asm_64.S) ------------------
    "gfni-evex": [
        ".text", "vgf2p8affineinvqb $0, %zmm1, %zmm2, %zmm3", "vgf2p8affineqb $0xc5, %zmm1, %zmm2, %zmm3",
        "vgf2p8affineinvqb $5, (%rax), %zmm2, %zmm3", "vgf2p8affineinvqb $5, (%rax){1to8}, %zmm2, %zmm3",
        "vgf2p8affineinvqb $5, %zmm17, %zmm2, %zmm30", "vgf2p8affineqb $1, 0x40(%rax), %zmm2, %zmm3{%k1}{z}",
        "vgf2p8affineqb $1, %xmm17, %xmm2, %xmm3", "vgf2p8affineqb $1, (%rax){1to4}, %ymm2, %ymm3",
        "vgf2p8mulb %zmm1, %zmm2, %zmm3", "vgf2p8mulb 0x40(%rax), %zmm2, %zmm3{%k2}", "vgf2p8mulb %ymm17, %ymm2, %ymm3",
        "vgf2p8affineinvqb $5, %ymm1, %ymm2, %ymm3", "vgf2p8mulb %xmm1, %xmm2, %xmm3", "gf2p8mulb %xmm1, %xmm2",
    ],
    # -- deferred .skip: the size is the expression's value in the final layout
    "alternative-2": [".text", *alternative(alternative(["jmp .Lskip"], 31), 13), ".Lskip:", "ret"],
    "alternative-3": [
        ".text", *alternative(alternative(alternative(["jmp .Lskip"], 7), 13), 9), ".byte 742b-740b", ".Lskip:", "ret",
    ],
    "alternative-far": [".text", *alternative(["jmp .Lskip"], 200), ".byte 742b-740b", ".Lskip:", "ret"],
    "skip-measures-skip": [
        ".text", "0:", "nop", "1:", ".skip (1b-0b)*5, 0x90", "2:", ".skip (2b-0b)*2, 0xcc", "3:", ".byte 3b-0b", "ret",
    ],
    "skip-measures-later-skip": [
        ".text", "0:", "nop", "1:", ".skip (3f-2f), 0x90", "2:", ".skip (4f-3f)*3, 0xcc", "3:", "nop", "nop", "4:",
        ".byte 4b-0b", "ret",
    ],
    "skips-at-one-offset": [
        ".text", "0:", "nop", "1:", ".skip (1b-0b)*3, 0xaa", ".skip (1b-0b)*2, 0xbb", "2:", ".byte 2b-1b", "ret",
    ],
    "skip-grows-jump-grows-skip": [
        ".text", "0:", "jmp 2f", "1:", ".skip (4f-3f), 0x90", "2:", "ret", "5:", ".skip (1b-0b)*4, 0xcc", "6:",
        ".byte 6b-5b", ".section .rodata", "3:", ".fill 130", "4:",
    ],
    "skip-measures-padding": [
        ".text", "0:", "nop", "1:", ".skip (3f-2f)*2, 0xcc", "2:", "nop", ".p2align 4, 0x11", "3:", ".byte 3b-0b", "ret",
    ],
    "skip-nobits": [
        ".text", "0:", "jmp 2f", "1:", ".skip (4f-3f), 0x90", "2:", "ret", ".section .rodata", "3:", ".fill 200", "4:",
        ".bss", "b0:", ".skip 100 - (1b-0b)", "b1:", ".skip (b1-b0)*2", "b2:", "nop", "b3:",
        ".data", ".long b1-b0, b2-b0, b3-b0",
    ],
    "skip-takes-the-place-of-padding": [
        ".text", "0:", ".fill 10, 1, 0x90", "1:", ".skip (1b-0b)*6, 0xcc", "jmp 9f", ".fill 120, 1, 0x90",
        ".p2align 6, 0x11", "9:", "ret",
    ],
    "skip-without-fixed-point": [".text", "0:", ".skip 5 - (1f - 0b), 0x90", "1:", "ret"],
    "skip-diverges": [".text", "0:", ".skip (1f - 0b) + 1, 0x90", "1:", "ret"],
    # Who binds a branch. A label of the branch's own section is not always
    # the assembler's to resolve: a weak definition can be replaced at link
    # time, an ifunc is reached through its PLT entry, and `sym@PLT` on a
    # global asks for the linker. Resolved here, `jmp hook` stayed with the
    # weak `hook` whatever the link provided.
    "branch-to-weak": BRANCHES_TO("hook", [".weak hook"]),
    "branch-to-global": BRANCHES_TO("hook", [".globl hook"]),
    "branch-to-hidden": BRANCHES_TO("hook", [".globl hook", ".hidden hook"]),
    "branch-to-protected-weak": BRANCHES_TO("hook", [".weak hook", ".protected hook"]),
    "branch-to-local": BRANCHES_TO("hook", []),
    "branch-to-ifunc": BRANCHES_TO("hook", [".type hook, @gnu_indirect_function"]),
    "branch-to-global-ifunc": BRANCHES_TO("hook", [".globl hook", ".type hook, @gnu_indirect_function"]),
    # The directive counts wherever it stands: GNU as binds once it has read
    # the file.
    "branch-to-weak-declared-last": BRANCHES_TO("hook", [], [".weak hook"]),
    "branch-to-hidden-declared-last": BRANCHES_TO("hook", [], [".globl hook", ".hidden hook"]),
    "branch-to-ifunc-declared-last": BRANCHES_TO("hook", [], [".type hook, @gnu_indirect_function"]),
    # Out of rel8 reach the jump is still the assembler's: no relocation
    # against a global label of the jump's own section.
    "far-branch-to-global": [
        ".text", ".globl far, far_hidden", ".hidden far_hidden", ".weak far_weak",
        "jmp far", "jne far", "jmp far+1", "jmp far_hidden@PLT", "jmp far@PLT", "jmp far_weak", "jmp far_local@PLT",
        ".fill 300, 1, 0x90", "far:", "ret", "far_hidden:", "ret", "far_weak:", "ret", "far_local:", "ret",
    ],
    # Another section's label: the section symbol for a local one, the
    # symbol and a PLT32 for one the linker may rebind.
    "branch-across-sections": [
        ".section .text.b,\"ax\"", ".globl og", ".weak ow", "ol:", "ret", "og:", "ret", "ow:", "ret",
        ".text", "jmp ol", "jne ol", "call ol", "jmp ol+3", "loop ol", "jrcxz og",
        "jmp og", "call og", "jmp ow", "call ow", "jmp later",
        ".section .text.b", "later:", "ret",
    ],
    # A constant in a mergeable section is named by its symbol: the linker
    # reads `section + addend` there as an entry's address, and the -4 of a
    # RIP-relative reference is not part of one.
    "merge-section-references": [
        ".section .rodata.cst16.k,\"aM\",@progbits,16",
        ".Lk1:", ".quad 0x1111111111111111, 0x1111111111111111",
        ".Lk2:", ".quad 0x2222222222222222, 0x2222222222222222",
        ".section .rodata.str1.1,\"aMS\",@progbits,1", ".Ls0:", ".string \"abc\"", ".Ls1:", ".string \"defg\"",
        ".text", "movdqa .Lk1(%rip), %xmm0", "movdqa .Lk2(%rip), %xmm1", "lea .Ls1(%rip), %rax",
        "lea .Ls1+1(%rip), %rax", "movl $.Ls1, %edi", "movl $.Ls1+2, %edi", "movq $.Lk2, %rdi", "ret",
        ".data", ".quad .Lk2, .Ls1, .Ls1+2", ".long .Ls1 - .",
    ],
}


def elf_section_bytes(path: Path, wanted=(".text", ".data")) -> dict:
    """What an ELF64 relocatable object says about the named sections.

    Their contents under the section's name, and under `.rela<name>` their
    relocations as sorted `(offset, type, symbol, addend)` -- a section
    symbol under its section's name. Two objects can agree in every byte
    and still ask the linker for different things: a branch it may rebind
    against one it may not, a constant named by its symbol against "twelve
    bytes into the entry before it".
    """
    data = path.read_bytes()
    if data[:4] != b"\x7fELF" or data[4] != 2:
        raise Failure(f"asm: {path.name} is not an ELF64 object")

    def field(at, width):
        return int.from_bytes(data[at : at + width], "little")

    shoff, shentsize, shnum, shstrndx = field(40, 8), field(58, 2), field(60, 2), field(62, 2)

    def header(index):
        base = shoff + index * shentsize
        return {
            "name": field(base, 4), "type": field(base + 4, 4), "offset": field(base + 24, 8),
            "size": field(base + 32, 8), "link": field(base + 40, 4), "info": field(base + 44, 4),
        }

    def text(table, at):
        start = table["offset"] + at
        return data[start : data.index(b"\0", start)].decode()

    headers = [header(index) for index in range(shnum)]
    names = [text(headers[shstrndx], h["name"]) for h in headers]
    found = {}
    for h, name in zip(headers, names):
        if name in wanted and h["type"] != 8:  # SHT_NOBITS has no contents
            found[name] = data[h["offset"] : h["offset"] + h["size"]]
        if h["type"] != 4 or names[h["info"]] not in wanted:  # SHT_RELA
            continue
        symtab = headers[h["link"]]
        strtab = headers[symtab["link"]]
        entries = []
        for at in range(h["offset"], h["offset"] + h["size"], 24):
            info = field(at + 8, 8)
            sym = symtab["offset"] + (info >> 32) * 24
            if data[sym + 4] & 0xF == 3:  # STT_SECTION
                symbol = names[field(sym + 6, 2)]
            else:
                symbol = text(strtab, field(sym, 4))
            addend = int.from_bytes(data[at + 16 : at + 24], "little", signed=True)
            entries.append((field(at, 8), info & 0xFFFFFFFF, symbol, addend))
        found[name] = sorted(entries)
    return found


RELOC_NAMES = {1: "64", 2: "PC32", 4: "PLT32", 10: "32", 11: "32S", 15: "PC8", 24: "PC64"}


def assemble(tool, source: Path, obj: Path):
    """Section contents, or None when `tool` refuses the source."""
    p = run([*tool, source, "-o", obj])
    return elf_section_bytes(obj) if p.returncode == 0 else None


def gnu_as(gcc: str):
    p = run([gcc, "-print-prog-name=as"])
    path = p.stdout.strip()
    if p.returncode != 0 or not path or shutil.which(path) is None:
        raise Failure(f"asm: {gcc} names no assembler ({path!r})")
    return [path, "--64"]


def check_asm(lccc: Path, gcc: str, tmp: Path, cases=None):
    reference = gnu_as(gcc)
    count = 0
    for name, lines in (cases or ASM_CASES).items():
        # A pair is (what GNU as gets, what LCCC gets): the self-test's way
        # of showing that a difference is seen.
        theirs, ours = lines if isinstance(lines, tuple) else (lines, lines)
        want_src, got_src = tmp / f"{name}.gas.s", tmp / f"{name}.lccc.s"
        want_src.write_text("\n".join(theirs) + "\n")
        got_src.write_text("\n".join(ours) + "\n")
        want = assemble(reference, want_src, tmp / f"{name}.gas.o")
        got = assemble([lccc, "-c"], got_src, tmp / f"{name}.lccc.o")
        if want != got:
            def show(result):
                if result is None:
                    return "refused"
                return "; ".join(
                    f"{section} " + (
                        content.hex(" ") if isinstance(content, bytes)
                        else " ".join(f"{off:#x}:{RELOC_NAMES.get(kind, kind)}:{sym}{add:+d}" for off, kind, sym, add in content)
                    )
                    for section, content in sorted(result.items())
                )
            raise Failure(
                f"asm: {name}: GNU as and LCCC disagree\n    source: " + " | ".join(ours)
                + f"\n    GNU as: {show(want)}\n    LCCC:   {show(got)}"
            )
        count += 1
    return count


WEAK_BRANCHES = """
	.text
	.weak hook
	.globl via_jmp, via_jcc, via_call
via_jmp:
	jmp hook
via_jcc:
	xorl %eax, %eax
	testl %eax, %eax
	je hook
	ret
via_call:
	call hook
	ret
hook:
	movl $1, %eax
	ret
	.section .note.GNU-stack,"",@progbits
"""

WEAK_BRANCHES_MAIN = """
#include <stdio.h>
int via_jmp(void), via_jcc(void), via_call(void);
int hook(void) { return 2; }
int main(void) { printf("%d %d %d\\n", via_jmp(), via_jcc(), via_call()); return 0; }
"""


def check_asm_link(lccc: Path, gcc: str, tmp: Path):
    """A weak definition and the branches to it share a section; the program
    defines the symbol again, strongly. Every branch has to reach that one."""
    source, main = tmp / "weak-branches.s", tmp / "weak-branches-main.c"
    source.write_text(WEAK_BRANCHES.lstrip("\n"))
    main.write_text(WEAK_BRANCHES_MAIN)
    seen = {}
    for who, tool in (("GNU as", gnu_as(gcc)), ("LCCC", [lccc, "-c"])):
        stem = tmp / f"weak-branches-{who.split()[-1].lower()}"
        obj, exe = stem.with_suffix(".o"), stem.with_suffix(".exe")
        p = run([*tool, source, "-o", obj])
        if p.returncode != 0:
            raise Failure(f"asm: {who} refuses the weak-branch source:\n{p.stderr}")
        p = run([gcc, "-O1", "-w", main, obj, "-o", exe])
        if p.returncode != 0:
            raise Failure(f"asm: the weak-branch program does not link with the {who} object:\n{p.stderr}")
        seen[who] = run([exe], timeout=30).stdout
    if seen["GNU as"] != "2 2 2\n":
        raise Failure(f"asm: the GNU as build of the weak-branch program prints {seen['GNU as']!r}")
    if seen["LCCC"] != seen["GNU as"]:
        raise Failure(
            "asm: a branch to a weak definition was bound at assembly time: via_jmp, via_jcc, via_call "
            f"return {seen['LCCC'].strip()} where the program's own `hook` returns 2"
        )
    return 1


# --------------------------------------------------------------------------
# self-test: the checks see planted faults
# --------------------------------------------------------------------------

PLANTED_DEAD_RELAY = """
    cmpl $5, %edi
    jae .LBB7
    jmpq *.Ljt_0(,%rdi,8)
.section .rodata
.Ljt_0:
    .quad .LBB7
    .quad .LBB3
.section .text,"ax",@progbits
.LBB2:
    jmp .LBB7
.LBB3:
    call say
.LBB7:
    ret
""".strip().split("\n")

PLANTED_LIVE = """
    testl %edi, %edi
    je .LBB2
    call abort
.LBB2:
#APP
    1: ud2
    .pushsection __bug_table,"aw" ; .long 1b - . ; .popsection
#NO_APP
    leaq .LBB9(%rip), %rax
    jmpq *%rax
.LBB9:
    ret
""".strip().split("\n")


def self_test(lccc: Path, gcc: str, tmp: Path):
    checks = 0
    dead = unreachable_instructions(PLANTED_DEAD_RELAY)
    if dead != ["jmp .LBB7"]:
        raise Failure(f"self-test: the relay behind a jump table was not reported: {dead}")
    if unreachable_instructions(PLANTED_LIVE):
        raise Failure(f"self-test: live code reported: {unreachable_instructions(PLANTED_LIVE)}")
    checks += 2
    # Each fix off again, each check on: the gate has to fail, and for the
    # reason the fix was made.
    planted = [
        ("switch_cover", check_reach, "nothing reaches"),
        ("byvalscalar", check_shape, "grows with the chain"),
    ]
    for passes, check, reason in planted:
        sub = tmp / f"planted-{passes}"
        sub.mkdir()
        try:
            check(lccc, sub, env={"CCC_DISABLE_PASSES": passes})
        except Failure as failure:
            if reason not in str(failure):
                raise Failure(f"self-test: without {passes} the gate failed for another reason:\n{failure}")
            checks += 1
        else:
            raise Failure(f"self-test: without {passes} the gate still passes: {check.__name__} checks nothing")
    # The assembler comparison sees one differing byte, and a source only
    # one of the two accepts.
    for name, pair in {
        "planted-byte": ([".data", ".byte 1"], [".data", ".byte 2"]),
        "planted-refusal": ([".data", ".byte 1"], [".data", ".byte \\zz"]),
        # The same bytes, another request to the linker.
        "planted-relocation": ([".text", "call ext"], [".text", "call other"]),
    }.items():
        try:
            check_asm(lccc, gcc, tmp, {name: pair})
        except Failure as failure:
            if "disagree" not in str(failure):
                raise Failure(f"self-test: {name} failed for another reason:\n{failure}")
            checks += 1
        else:
            raise Failure(f"self-test: {name} was not reported: check_asm checks nothing")
    return checks


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--only", action="append", choices=["run", "reach", "shape", "abi", "diag", "asm"])
    args = parser.parse_args()
    lccc = Path(args.lccc).resolve()
    if not lccc.exists():
        print(f"check_kernel_driver_shapes: no compiler at {lccc}", file=sys.stderr)
        return 2
    if shutil.which(args.gcc) is None:
        print(f"check_kernel_driver_shapes: reference compiler {args.gcc} not found", file=sys.stderr)
        return 2
    wanted = args.only or ["run", "reach", "shape", "abi", "diag", "asm"]
    with tempfile.TemporaryDirectory(prefix="lccc-kernel-driver-shapes-") as td:
        tmp = Path(td)
        try:
            report = []
            if "run" in wanted:
                m32 = can_run_m32(lccc, tmp)
                rows = check_run(lccc, args.gcc, tmp, m32)
                report.append(f"run {rows}" + ("" if m32 else " (no i386 runtime: x86-64 only)"))
            if "reach" in wanted:
                report.append(f"reach {check_reach(lccc, tmp)} functions")
            if "shape" in wanted:
                report.append(f"shape {check_shape(lccc, tmp)}")
            if "abi" in wanted:
                report.append(f"abi {check_abi(lccc, args.gcc, tmp)}")
            if "diag" in wanted:
                report.append(f"diag {check_diag(lccc, args.gcc, tmp)}")
            if "asm" in wanted:
                report.append(f"asm {check_asm(lccc, args.gcc, tmp) + check_asm_link(lccc, args.gcc, tmp)}")
            if args.self_test:
                report.append(f"self-test {self_test(lccc, args.gcc, tmp)} planted faults seen")
        except Failure as failure:
            print(f"kernel-driver-shapes: FAIL\n{failure}", file=sys.stderr)
            return 1
    print("kernel-driver-shapes: PASS (" + ", ".join(report) + ")")
    return 0


if __name__ == "__main__":
    sys.exit(main())
