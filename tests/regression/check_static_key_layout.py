#!/usr/bin/env python3
"""A static key costs what its `nop` costs.

The kernel tests a configuration switch with `if (static_branch_unlikely(&key))`
(include/linux/jump_label.h, arch/x86/include/asm/jump_label.h).  The test is
an asm goto that holds one five-byte `nop` and an entry in `__jump_table`;
enabling the key patches the `nop` into a `jmp` to the label.  The whole point
is the path with the key off: execution falls through the `nop` into the code
behind the `if`, and nothing else is paid for the test.

Four things made LCCC pay more, each at every such test:

* the address of the key was loaded into a register ahead of the `nop`
  (`leaq key(%rip), %rax`), for an operand that is printed into the table;
* the block the label names was laid out behind the asm, in source order, so
  the path that falls through took a `jmp` over it -- inside loops, once per
  iteration;
* an immediate operand made the statement a call for the register allocator,
  so whatever was live across the test moved to a callee-saved register or
  the stack, and a function that only spins on `cpu_relax()` saved three
  registers for its parameters;
* behind `static_branch_likely()` the two ways out met in a 0/1 value that
  was materialized, tested again and fed to a `cmov`.

GCC 16.2 and Clang 23.1.5 emit the `nop` and the straight-line code.  This
gate holds LCCC to the same, on the assembly it prints at -O1, -O2 and -O3 for
x86-64 and i686, and runs the program with the keys off (`nop`) and on (`jmp`)
at every level to see that both ways still compute the same as plain C.

    check_static_key_layout.py --lccc PATH     the gate
    check_static_key_layout.py --self-test     the checks see planted faults
    check_static_key_layout.py --emit          print the C source
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

SOURCE = r"""
/* Static keys as the kernel writes them, with the keys off (a nop) or, with
 * -DKEYS_ON, on (the jump the nop is patched into). */
extern int printf(const char *, ...);

struct static_key {
    int enabled;
};
struct static_key the_key, other_key;

#if defined(__x86_64__)
#define KEY_WORD ".quad"
#define KEY_ALIGN "8"
#else
#define KEY_WORD ".long"
#define KEY_ALIGN "4"
#endif
#ifdef KEYS_ON
#define KEY_INSN "jmp %l[l_yes]"
#else
#define KEY_INSN ".byte 0x0f,0x1f,0x44,0x00,0x00"
#endif

static inline __attribute__((always_inline)) _Bool
arch_static_branch(struct static_key *const key, const _Bool branch)
{
    __asm__ goto("1:" KEY_INSN "\n\t"
                 ".pushsection __jump_table, \"aw\"\n\t"
                 ".balign " KEY_ALIGN "\n\t"
                 ".long 1b - .\n\t"
                 ".long %l[l_yes] - .\n\t"
                 KEY_WORD " %c0 + %c1 - .\n\t"
                 ".popsection\n\t"
                 : : "i"(key), "i"(branch) : : l_yes);
    return 0;
l_yes:
    return 1;
}

#define likely(x) __builtin_expect(!!(x), 1)
#define unlikely(x) __builtin_expect(!!(x), 0)
#define static_branch_unlikely(key) unlikely(arch_static_branch(key, 0))
#define static_branch_likely(key) likely(!arch_static_branch(key, 1))
#ifdef KEYS_ON
#define KEY_IS_ON 1
#else
#define KEY_IS_ON 0
#endif

static volatile int zero;
static int slow_calls;
__attribute__((noinline)) void slow(void)
{
    slow_calls += 1 + zero;
}

/* if (static_branch_unlikely(&key)) slow(); */
__attribute__((noinline)) int unlikely_call(int x)
{
    if (static_branch_unlikely(&the_key))
        slow();
    return x + 1;
}

/* Two keys in a row. */
__attribute__((noinline)) int two_keys(int x)
{
    if (static_branch_unlikely(&the_key))
        slow();
    x += 3;
    if (static_branch_unlikely(&other_key))
        slow();
    return x + 1;
}

/* A tracepoint in a loop. */
__attribute__((noinline)) int in_loop(const int *p, int n)
{
    int sum = 0;
    for (int i = 0; i < n; i++) {
        if (static_branch_unlikely(&the_key))
            slow();
        sum += p[i];
    }
    return sum;
}

/* if (static_branch_likely(&key)) x = x * 3 + 1;  -- no call anywhere. */
__attribute__((noinline)) int leaf_likely(int x)
{
    if (static_branch_likely(&the_key))
        x = x * 3 + 1;
    return x;
}

/* while (!*flag) cpu_relax(); */
__attribute__((noinline)) int spin(volatile int *flag, int a, int b)
{
    while (!*flag)
        __asm__ __volatile__("rep; nop" : : : "memory");
    return a + b;
}

int main(void)
{
    static const int data[4] = { 1, 2, 3, 4 };
    volatile int flag = 1;
    int a = unlikely_call(5 + zero);
    int b = two_keys(5 + zero);
    int c = in_loop(data, 4 + zero);
    int d = leaf_likely(5 + zero);
    int e = spin(&flag, 2, 3);
    int calls = slow_calls;
    /* What plain C computes for the same switch. */
    int want_calls = KEY_IS_ON ? 1 + 2 + 4 : 0;
    int want_d = KEY_IS_ON ? 5 : 16;
    printf("%d %d %d %d %d calls=%d\n", a, b, c, d, e, calls);
    if (a == 6 && b == 9 && c == 10 && d == want_d && e == 5 && calls == want_calls) {
        printf("static keys: ok\n");
        return 0;
    }
    printf("static keys: MISMATCH (want 6 9 10 %d 5 calls=%d)\n", want_d, want_calls);
    return 1;
}
"""

FUNCTION = re.compile(r"^([A-Za-z_][\w.$]*):\s*$")
SKIPPED = ("#", ".cfi", ".loc", ".file", ".p2align")


def functions(asm: str) -> dict[str, list[str]]:
    """Function name -> its lines, stripped, without blanks and comments."""
    out: dict[str, list[str]] = {}
    name = None
    for raw in asm.split("\n"):
        line = raw.strip()
        match = FUNCTION.match(line)
        if match and not line.startswith(".L"):
            name = match.group(1)
            out[name] = []
            continue
        if line.startswith(".size "):
            name = None
            continue
        if name is not None and line and not line.startswith(("# LCCC",)):
            out[name].append(line)
    return out


def is_instruction(line: str) -> bool:
    return not line.startswith((".", "#")) and not line.endswith(":")


def regions(lines: list[str]):
    """(start, end) index pairs of the #APP .. #NO_APP regions."""
    start = None
    for index, line in enumerate(lines):
        if line == "#APP":
            start = index
        elif line == "#NO_APP" and start is not None:
            yield start, index
            start = None


def outside_regions(lines: list[str]) -> list[str]:
    inside = set()
    for start, end in regions(lines):
        inside.update(range(start, end + 1))
    return [line for index, line in enumerate(lines) if index not in inside]


def check_assembly(asm: str, label: str, register_arguments: bool = True) -> list[str]:
    """The faults of one assembly listing, as sentences.

    `register_arguments`: the target passes arguments in registers (x86-64),
    so a function that calls nothing has no reason to save any.  i686 takes
    them from the stack and homes them in callee-saved registers with or
    without an asm in the function; there only the flag test is checked.
    """
    faults: list[str] = []
    funcs = functions(asm)
    wanted = ("unlikely_call", "two_keys", "in_loop", "leaf_likely", "spin")
    for name in wanted:
        if name not in funcs:
            faults.append(f"{label}: function `{name}` is missing from the output")
    if faults:
        return faults

    # 1. Nothing is paid behind the nop: the instruction after an asm goto is
    #    never a jump over the block its label names.
    for name in ("unlikely_call", "two_keys", "in_loop", "leaf_likely"):
        lines = funcs[name]
        found = 0
        for _, end in regions(lines):
            found += 1
            follower = next((l for l in lines[end + 1 :] if is_instruction(l)), "")
            if re.match(r"^jmp\s", follower):
                faults.append(
                    f"{label}: `{name}` jumps right behind a static key (`{follower}`): "
                    "the path that falls through the nop pays for the label's block"
                )
        if not found:
            faults.append(f"{label}: `{name}` lost its static key")

    # 2. The key's address is printed into the table and loaded nowhere.
    for name in ("unlikely_call", "two_keys", "in_loop", "leaf_likely"):
        for line in outside_regions(funcs[name]):
            if is_instruction(line) and re.search(r"\b(the_key|other_key)\b", line):
                faults.append(
                    f"{label}: `{name}` computes the key's address (`{line}`) "
                    "for an operand that is an immediate"
                )

    # 3. The call a key guards is out of line: behind the function's return.
    lines = funcs["unlikely_call"]
    instructions = [l for l in lines if is_instruction(l)]
    calls = [i for i, l in enumerate(instructions) if re.match(r"^call\s+slow\b", l)]
    returns = [i for i, l in enumerate(instructions) if re.match(r"^ret\b", l)]
    if not calls:
        faults.append(f"{label}: `unlikely_call` lost its call")
    elif not returns or calls[0] < returns[0]:
        faults.append(
            f"{label}: `unlikely_call` has the guarded call in line, ahead of its return"
        )

    # 4. In a loop the call is behind the back edge: an iteration with the key
    #    off takes the back edge and no other jump.
    lines = funcs["in_loop"]
    instructions = [l for l in lines if is_instruction(l)]
    calls = [i for i, l in enumerate(instructions) if re.match(r"^call\s+slow\b", l)]
    keys = [i for i, l in enumerate(instructions) if l.startswith("1:")]
    if calls and keys:
        behind = instructions[keys[0] + 1 : calls[0]]
        if not any(re.match(r"^j(?!mp)[a-z]+\s", l) for l in behind):
            faults.append(
                f"{label}: `in_loop` has the guarded call between the key and the loop's back edge"
            )
    elif not calls:
        faults.append(f"{label}: `in_loop` lost its call")

    # 5. A function that only tests a key or spins keeps its parameters where
    #    they arrived and tests no flag value: the asm names no register.
    saves = r"push|sub\w*\s+\$\d+,\s*%[er]sp"
    flag_value = r"cmov|set|test"
    for name, banned in (
        ("leaf_likely", f"^({saves}|{flag_value})" if register_arguments else f"^({flag_value})"),
        ("spin", f"^({saves})" if register_arguments else None),
    ):
        if banned is None:
            continue
        for line in funcs[name]:
            if is_instruction(line) and re.match(banned, line):
                faults.append(
                    f"{label}: `{name}` contains `{line}`: a statement that names no "
                    "register was treated as one that destroys them, or the two ways "
                    "out of the key were merged into a value"
                )
    return faults


def self_test() -> int:
    """Each check reports the fault it is there for, and nothing on clean text."""
    clean = """
unlikely_call:
    leal 1(%rdi), %eax
#APP
    1:.byte 0x0f,0x1f,0x44,0x00,0x00
    .long .LBB4 - .
#NO_APP
    ret
.LBB4:
    call slow
    jmp .LBB3
.size unlikely_call, .-unlikely_call
two_keys:
#APP
    1:.byte 0x0f,0x1f,0x44,0x00,0x00
    .long .LBB8 - .
#NO_APP
    ret
.size two_keys, .-two_keys
in_loop:
.LBB20:
#APP
    1:.byte 0x0f,0x1f,0x44,0x00,0x00
    .long .LBB23 - .
#NO_APP
    addl (%rdi), %eax
    jl .LBB20
    ret
.LBB23:
    call slow
    jmp .LBB20
.size in_loop, .-in_loop
leaf_likely:
#APP
    1:.byte 0x0f,0x1f,0x44,0x00,0x00
    .long .LBB29 - .
#NO_APP
    leal 1(%rdi, %rdi, 2), %eax
    ret
.size leaf_likely, .-leaf_likely
spin:
    ret
.size spin, .-spin
"""
    faults = check_assembly(clean, "clean")
    if faults:
        print("self-test FAILED: clean text reported", faults)
        return 1
    plants = {
        "jumps right behind": clean.replace(
            "#NO_APP\n    ret\n.LBB4:", "#NO_APP\n    jmp .LBB3\n.LBB4:"
        ),
        "computes the key's address": clean.replace(
            "    leal 1(%rdi), %eax\n#APP", "    leaq the_key(%rip), %rax\n#APP"
        ),
        "guarded call in line": clean.replace(
            "#NO_APP\n    ret\n.LBB4:\n    call slow\n    jmp .LBB3",
            "#NO_APP\n.LBB4:\n    call slow\n    ret",
        ),
        "between the key and the loop's back edge": clean.replace(
            "#NO_APP\n    addl (%rdi), %eax\n    jl .LBB20\n    ret\n.LBB23:\n    call slow\n    jmp .LBB20",
            "#NO_APP\n    nop\n.LBB23:\n    call slow\n    addl (%rdi), %eax\n    jl .LBB20\n    ret",
        ),
        "`leaf_likely` contains": clean.replace(
            "    leal 1(%rdi, %rdi, 2), %eax\n    ret",
            "    testb %r11b, %r11b\n    cmovel %r9d, %eax\n    ret",
        ),
        "`spin` contains": clean.replace("spin:\n    ret", "spin:\n    pushq %rbx\n    ret"),
        "is missing": clean.replace("leaf_likely:", "leaf_unlikely:"),
    }
    failed = 0
    for needle, text in plants.items():
        if text == clean:
            print(f"self-test FAILED: the plant for {needle!r} changed nothing")
            failed += 1
            continue
        faults = check_assembly(text, "plant")
        if not any(needle in fault for fault in faults):
            print(f"self-test FAILED: planted {needle!r}, got {faults}")
            failed += 1
    if failed:
        return 1
    print(f"self-test ok: {len(plants)} planted faults reported, clean text clean")
    return 0


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, errors="replace", **kw)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", help="compiler under test")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--emit", action="store_true", help="print the C source")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()
    if args.emit:
        sys.stdout.write(SOURCE)
        return 0
    status = 0
    if args.self_test:
        status |= self_test()
    if not args.lccc:
        if not args.self_test:
            ap.error("--lccc, --self-test or --emit")
        return status

    failures = 0
    layout_rows = 0
    run_rows = 0
    skipped = 0
    with tempfile.TemporaryDirectory(prefix="static-key-") as tmp:
        src = Path(tmp) / "static_key.c"
        src.write_text(SOURCE)
        # The layout, on what the compiler prints with the keys off.
        for bits in ("-m64", "-m32"):
            for level in ("-O1", "-O2", "-O3"):
                out = Path(tmp) / "static_key.s"
                step = run([args.lccc, bits, level, "-fno-pie", "-S", str(src), "-o", str(out)])
                label = f"{bits} {level}"
                if step.returncode != 0:
                    if bits == "-m32" and "sysroot" in step.stderr.lower():
                        skipped += 1
                        continue
                    print(f"FAIL {label}: does not compile\n{step.stderr.strip()[-600:]}")
                    failures += 1
                    continue
                layout_rows += 1
                faults = check_assembly(
                    out.read_text(errors="replace"), label, register_arguments=bits == "-m64"
                )
                for fault in faults:
                    print("FAIL", fault)
                failures += bool(faults)
                if args.verbose and not faults:
                    print(f"ok   {label}: layout")
        # Both ways out, run: the nop and the jump it is patched into.
        for bits in ("-m64", "-m32"):
            for level in ("-O0", "-O1", "-O2", "-O3", "-Os"):
                for keys in ((), ("-DKEYS_ON",)):
                    exe = Path(tmp) / "static_key"
                    label = " ".join((bits, level) + keys)
                    step = run([args.lccc, bits, level, *keys, "-w", str(src), "-o", str(exe)])
                    if step.returncode != 0:
                        if bits == "-m32":
                            # no 32-bit C library to link against on this host
                            skipped += 1
                            continue
                        print(f"FAIL {label}: does not build\n{step.stderr.strip()[-600:]}")
                        failures += 1
                        continue
                    try:
                        result = run([str(exe)], timeout=20)
                    except subprocess.TimeoutExpired:
                        print(f"FAIL {label}: still running after 20 s")
                        failures += 1
                        continue
                    except OSError:
                        skipped += 1  # no 32-bit loader on this host
                        continue
                    run_rows += 1
                    if result.returncode != 0 or "static keys: ok" not in result.stdout:
                        print(f"FAIL {label}: {result.stdout.strip()} (exit {result.returncode})")
                        failures += 1
                    elif args.verbose:
                        print(f"ok   {label}: {result.stdout.splitlines()[0]}")
    note = f", {skipped} skipped (no 32-bit runtime)" if skipped else ""
    print(
        f"static key layout: {layout_rows} listings and {run_rows} runs checked, "
        f"{failures} failed{note}"
    )
    return 1 if failures or status else 0


if __name__ == "__main__":
    sys.exit(main())
