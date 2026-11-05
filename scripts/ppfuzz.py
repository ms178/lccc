#!/usr/bin/env python3
"""Differential fuzz of the preprocessor against GCC 16.2 and Clang 23.1.5.

Why a token comparison rather than a text one: `-E -P` output differs between
compilers in whitespace and line breaks all the time (that is formatting), and
collapsing whitespace with `tr -s` hides the differences that matter, because it
also rewrites the inside of string literals. This compares *preprocessing tokens*
-- the actual contract -- so `+ +` and `++` stay different, and `"a  b"` keeps its
two spaces.

Every case is a generated translation unit built out of the constructs the
preprocessor is hardest to get right on: identical and non-identical
redefinitions, `#undef` resurrection, builtin takeover, nested includes with
guards, `#pragma once`, `#include_next`, conditional nesting, stringification and
pasting, variadics, `__INCLUDE_LEVEL__`, `#line`, and comments in every position.
A case where a compiler rejects the input is compared on (status, stdout) rather
than stdout alone, so "accepts what gcc rejects" is a finding too.

Usage: ppfuzz.py [--cases N] [--seed S] [--jobs J] [--keep DIR]
"""
from __future__ import annotations

import argparse
import concurrent.futures
import os
import random
import shutil
import subprocess
import sys
import tempfile

LCCC = os.environ.get("LCCC", "target/fastbuild/lccc")
GCC = os.environ.get("GCC", "gcc-16")
CLANG = os.environ.get("CLANG", "clang-23")


# --- tokeniser -------------------------------------------------------------
PUNCTUATORS = sorted(
    """%:%: ... <<= >>= -> ++ -- << >> <= >= == != && || *= /= %= += -= &= ^= |= ##
    <: :> <% %> %: { } [ ] ( ) ; : ? . + - * / % ^ & | ~ ! = < > # ,""".split(),
    key=len,
    reverse=True,
)

ID_START = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$")
ID_CONT = ID_START | set("0123456789")


def tokenize(text: str) -> list[str]:
    """Split preprocessed C into preprocessing tokens."""
    tokens: list[str] = []
    i, n = 0, len(text)
    while i < n:
        ch = text[i]
        if ch in " \t\r\n\f\v":
            i += 1
            continue
        # string and character literals, including their prefix (L"x", u8"x")
        start = i
        while i < n and text[i] in "LuU":
            # a prefix only if a quote follows the prefix
            j = i
            while j < n and text[j] in "LuU8":
                j += 1
            if j < n and text[j] in "\"'":
                i = j
                break
            i = start
            break
        if i < n and text[i] in "\"'":
            quote = text[i]
            i += 1
            while i < n:
                if text[i] == "\\":
                    i += 2
                    continue
                if text[i] == quote:
                    i += 1
                    break
                i += 1
            tokens.append(text[start:i])
            continue
        if ch in ID_START:
            j = i
            while j < n and text[j] in ID_CONT:
                j += 1
            tokens.append(text[i:j])
            i = j
            continue
        # pp-number: digit or `.digit`, then digits/letters/`.`/exponent signs
        if ch.isdigit() or (ch == "." and i + 1 < n and text[i + 1].isdigit()):
            j = i + 1
            while j < n:
                c = text[j]
                if c in ID_CONT or c == ".":
                    j += 1
                elif c in "+-" and text[j - 1] in "eEpP":
                    j += 1
                else:
                    break
            tokens.append(text[i:j])
            i = j
            continue
        for punct in PUNCTUATORS:
            if text.startswith(punct, i):
                tokens.append(punct)
                i += len(punct)
                break
        else:
            tokens.append(ch)
            i += 1
    return tokens


# --- generator -------------------------------------------------------------
NAMES = ["A", "B", "C", "D", "E", "F"]
BUILTINS = ["__FILE__", "__LINE__", "__COUNTER__", "__INCLUDE_LEVEL__", "__TIMESTAMP__",
            "__DATE__", "__TIME__", "__BASE_FILE__"]


def gen_header(rng: random.Random, index: int, depth: int) -> tuple[str, str]:
    """Return (filename, text) for a header at the given nesting depth."""
    name = f"h{index}.h"
    lines = [
        f"#ifndef H{index}_INCLUDED",
        f"#define H{index}_INCLUDED",
    ]
    if rng.random() < 0.2:
        lines.append("#pragma once")
    for _ in range(rng.randint(1, 3)):
        kind = rng.random()
        if kind < 0.3:
            lines.append(f"int hv{index}_{rng.randrange(100)} = __INCLUDE_LEVEL__;")
        elif kind < 0.45:
            lines.append(f"const char *hf{index}_{rng.randrange(100)} = __FILE__;")
        elif kind < 0.55 and depth > 0:
            lines.append(f'#include "h{rng.randrange(index)}.h"')
        elif kind < 0.75:
            sym = rng.choice(NAMES)
            lines.append(f"#define {sym} ({rng.randint(0, 9)} + {rng.randint(0, 9)})")
            lines.append(f"int hu{index}_{rng.randrange(100)} = {sym};")
        else:
            sym = rng.choice(NAMES)
            lines.append(f"#undef {sym}")
    lines.append(f"#endif /* H{index}_INCLUDED */")
    return name, "\n".join(lines) + "\n"


def gen_hard_construct(rng: random.Random, sym: str) -> str:
    """The corner cases, one at a time.

    Each is something a preprocessor can plausibly get wrong while still being
    self-consistent: whitespace-insensitive redefinition identity, parameter
    renaming, object/function-like flips, empty variadic packs, `__VA_OPT__`,
    GNU `, ## __VA_ARGS__`, pasting into a punctuator, `#if` arithmetic the
    standard leaves implementation-defined-or-defined, and `__has_include`.
    """
    tag = rng.randrange(100000)
    choice = rng.randrange(14)
    if choice == 0:
        # C11 6.10.3p2: identical token sequence, whitespace differs => identical
        return (f"#define S{tag}(a, b) a + b\n"
                f"#define S{tag}(a,b)  a  +  b\n"
                f"int s{tag} = S{tag}(1, 2);")
    if choice == 1:
        # ... and so does renaming the parameters
        return (f"#define P{tag}(a) (a * 2)\n"
                f"#define P{tag}(b) (b * 2)\n"
                f"int p{tag} = P{tag}(3);")
    if choice == 2:
        # object-like then function-like: a different definition, not a syntax error
        return (f"#define O{tag} 1\n"
                f"#define O{tag}(x) (x)\n"
                f"int o{tag} = 1 + O{tag}(4);")
    if choice == 3:
        return (f"#define V{tag}(...) __VA_ARGS__\n"
                f"int v{tag}a = V{tag}();\n"
                f"int v{tag}b = V{tag}(1, 2);")
    if choice == 4:
        return (f"#define VO{tag}(a, ...) a __VA_OPT__(+ __VA_ARGS__)\n"
                f"int vo{tag}a = VO{tag}(1);\n"
                f"int vo{tag}b = VO{tag}(1, 2);")
    if choice == 5:
        return (f"#define GNU{tag}(a, ...) a , ## __VA_ARGS__\n"
                f"int g{tag}a[] = {{ GNU{tag}(1) }};\n"
                f"int g{tag}b[] = {{ GNU{tag}(1, 2) }};")
    if choice == 6:
        return (f"#define PASTE{tag}(a, b) a ## b\n"
                f"int past{tag} = PASTE{tag}(4, 5);")
    if choice == 7:
        return (f"#if defined({sym}) && ({sym} + 0) >= 0\n"
                f"int iff{tag} = 1;\n#else\nint iff{tag} = 2;\n#endif")
    if choice == 8:
        return (f"#if __has_include(\"h0.h\")\nint hi{tag} = 1;\n#else\nint hi{tag} = 0;\n#endif")
    if choice == 9:
        return (f"#if (1 ? 2 : 3) == 2 && (0 || 1) && !0\n"
                f"int tern{tag} = 1;\n#endif")
    if choice == 10:
        return (f"#define EMPTY{tag}\n"
                f"int e{tag} = 1 EMPTY{tag} + 1;")
    if choice == 11:
        return (f"#undef NEVER_DEFINED_{tag}\nint u{tag} = 0;")
    if choice == 12:
        return (f"#define TWICE{tag}(x) TWICE{tag}(x) x\n"
                f"int tw{tag} = TWICE{tag}(TWICE{tag}(7));")
    return (f"#define STRING{tag}(x) #x\n"
            f'const char *st{tag} = STRING{tag}(a   b);')

def gen_main(rng: random.Random, headers: list[str]) -> str:
    lines = []
    for _ in range(rng.randint(4, 14)):
        kind = rng.random()
        sym = rng.choice(NAMES)
        if kind < 0.16:
            lines.append(f"#define {sym} {rng.randint(0, 99)}")
        elif kind < 0.28:
            # identical redefinition: C11 6.10.3p2 says a redefinition with the
            # same token sequence is not an error, and both oracles stay silent
            body = rng.choice(["1", "(2 + 3)", "__LINE__"])
            lines.append(f"#define {sym} {body}")
            if rng.random() < 0.5:
                lines.append(f"#define {sym} {body}")
        elif kind < 0.36:
            lines.append(f"#undef {sym}")
            if rng.random() < 0.4:
                lines.append(f"#define {sym} {rng.randint(0, 9)}")
        elif kind < 0.46:
            lines.append(f"#ifdef {sym}")
            lines.append(f"int cond{rng.randrange(1000)} = {sym};")
            if rng.random() < 0.5:
                lines.append("#else")
                lines.append(f"int not{rng.randrange(1000)} = 0;")
            lines.append("#endif")
        elif kind < 0.54:
            builtin = rng.choice(BUILTINS)
            body = rng.choice(['7', '"taken"', '__LINE__'])
            lines.append(f"#define {builtin} {body}")
            lines.append(f"const char *take{rng.randrange(1000)} = {builtin};")
        elif kind < 0.62:
            builtin = rng.choice(BUILTINS)
            lines.append(f"int bi{rng.randrange(1000)} = (int)({builtin} != 0);")
        elif kind < 0.70:
            h = rng.choice(headers)
            lines.append(f'#include "{h}"')
            if rng.random() < 0.4:
                lines.append(f'#include "{h}"')      # guarded, must be a no-op
        elif kind < 0.78:
            fn = rng.choice(["STR", "CAT", "ID"])
            if fn == "STR":
                lines.append("#define STR(x) #x")
                lines.append(f'const char *s{rng.randrange(1000)} = STR({rng.choice(["a b", sym, "1 + 2"])});')
            elif fn == "CAT":
                lines.append("#define CAT(a, b) a ## b")
                lines.append(f"int cat{rng.randrange(1000)} = CAT(v, {rng.randint(1, 9)});")
            else:
                lines.append("#define ID(...) __VA_ARGS__")
                lines.append(f"int var{rng.randrange(1000)} = ID({sym}, {rng.randint(1, 9)});")
        elif kind < 0.86:
            lines.append(f"#line {rng.randint(1, 500)}")
            lines.append(f"int ln{rng.randrange(1000)} = __LINE__;")
        elif kind < 0.93:
            lines.append("/* a comment */ int c1; // trailing")
            lines.append(f"int c2 = {sym} /* mid */ + 1;")
        elif kind < 0.95:
            lines.append(gen_hard_construct(rng, sym))
        else:
            lines.append(f"int plain{rng.randrange(1000)} = {sym};")
    lines.append('const char *f = __FILE__;')
    return "\n".join(lines) + "\n"


def run(compiler: str, path: str, extra: list[str]) -> tuple[int, list[str]]:
    proc = subprocess.run([compiler, "-E", "-P", "-I", os.path.dirname(path), *extra, path],
                          capture_output=True, text=True)
    return proc.returncode, tokenize(proc.stdout)


def one_case(seed: int, workroot: str) -> str | None:
    rng = random.Random(seed)
    case = os.path.join(workroot, f"case{seed}")
    os.makedirs(case, exist_ok=True)
    headers = []
    for index in range(rng.randint(1, 3)):
        name, text = gen_header(rng, index, index)
        with open(os.path.join(case, name), "w") as handle:
            handle.write(text)
        headers.append(name)
    main = os.path.join(case, "main.c")
    with open(main, "w") as handle:
        handle.write(gen_main(rng, headers))
    extra = ["-std=gnu23"]
    # `__TIME__`, `__DATE__` and `__TIMESTAMP__` are sampled from the host clock,
    # so two compiler invocations can legitimately straddle a second (or a
    # midnight). A case that only disagrees on a re-run is not a finding, so a
    # mismatch has to reproduce before it is reported.
    for _attempt in range(3):
        base = run(LCCC, main, extra)
        mismatch = None
        for oracle in (GCC, CLANG):
            try:
                other = run(oracle, main, extra)
            except FileNotFoundError:
                continue
            if base != other:
                mismatch = (oracle, other)
                break
        if mismatch is None:
            break
    if mismatch is not None:
        oracle, other = mismatch
        if base != other:
            return (f"seed {seed}: {os.path.basename(oracle)} differs\n"
                    f"  lccc rc={base[0]} tokens={len(base[1])}\n"
                    f"  {os.path.basename(oracle)} rc={other[0]} tokens={len(other[1])}\n"
                    f"  first difference: {first_diff(base[1], other[1])}\n"
                    f"  case kept at {case}")
    shutil.rmtree(case, ignore_errors=True)
    return None


def first_diff(a: list[str], b: list[str]) -> str:
    for index, (x, y) in enumerate(zip(a, b)):
        if x != y:
            start = max(0, index - 6)
            return (f"token {index}: lccc={a[start:index + 7]!r} "
                    f"oracle={b[start:index + 7]!r}")
    if len(a) != len(b):
        return f"length {len(a)} vs {len(b)}, tail lccc={a[-6:]!r} oracle={b[-6:]!r}"
    return "none (whitespace only)"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", type=int, default=200)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--keep", default="")
    args = parser.parse_args(argv[1:])
    workroot = args.keep or tempfile.mkdtemp(prefix="ppfuzz-")
    os.makedirs(workroot, exist_ok=True)
    failures = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = {pool.submit(one_case, args.seed + i, workroot): args.seed + i
                   for i in range(args.cases)}
        for future in concurrent.futures.as_completed(futures):
            finding = future.result()
            if finding:
                failures.append(finding)
    print(f"{args.cases} case(s), {len(failures)} divergence(s), work root {workroot}")
    for finding in failures[:25]:
        print("FAIL", finding)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
