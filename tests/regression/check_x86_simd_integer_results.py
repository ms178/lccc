#!/usr/bin/env python3
"""The integer a SIMD operation returns, and the copies around it (x86-64).

`_mm_movemask_epi8(_mm_cmpeq_epi8(a, b)) == 0xffff` is how SIMD code asks
whether two blocks are equal. lccc answered in twelve instructions, with a
frame, a store and a compare against that store; GCC 16.2 and Clang 23.1.5
take six. Four causes, each with its own remedy, and this gate holds all
four:

1. the result of a mask, movemask, extract, conversion or test operation
   had no register to live in (`regalloc`: `X86_64_INTEGER_RESULTS`), so
   the compare behind it read the slot back;
2. a result that does live in a register was computed in `%rax` and copied
   there (`relay_and_lea::vector_to_gp_producer` retargets the producer);
3. the copies between a register-valued vector operation and one that
   takes its operand through a pointer appeared after the last turn of the
   passes that remove them, and `pmovmskb`/`movmskps` read as every vector
   register at once (`peephole` phase 3, `fp_liveness`);
4. a 512-bit mask compare loaded its second source into `%zmm1` first
   (`mask_cmp_source_512`), and the `kmov` behind it read as a use of the
   register it writes (`helpers::is_read_modify_write`).

What is checked, per function of the probe below:

* nothing touches the stack: no frame, no slot;
* no vector register is copied to another;
* a `vpcmp` names no `%zmm1`: its second source is where it was;
* it is no longer than GCC's function, give or take what the table says
  and why (GCC is measured here, not remembered);
* the functions of `OPEN` are recorded at the length they have: a 256-bit
  value that goes through a slot stays there. They may not get longer, and
  when they get shorter the record has to follow;
* its values are GCC's: the probe is built by both and run, where the CPU
  has what the group is built for (/proc/cpuinfo). Elsewhere the group is
  built and its shapes are checked, and the gate says it ran nothing.

Usage: check_x86_simd_integer_results.py --lccc BIN [--gcc gcc-16] [--cpu-flags "..."]
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile

PROBLEMS: list[str] = []

STACK = re.compile(r"\(%r[sb]p\)|^(?:push|pop)[qlw]?\s|%rsp\b")
VECTOR_COPY = re.compile(
    r"^v?mov(?:ap[sd]|up[sd]|dq[au](?:8|16|32|64)?|s[sd]|q)\s+%[xyz]mm\d+,\s*%[xyz]mm\d+$")

# name -> (body, slack): how many instructions more than GCC's the function
# may be, and the table below it says where each one comes from.
SSE2 = {
    "equal16": ("int equal16(__m128i a, __m128i b)\n"
                "{ return _mm_movemask_epi8(_mm_cmpeq_epi8(a, b)) == 0xffff; }", 0),
    "mask16": ("int mask16(__m128i a, __m128i b)\n"
               "{ return _mm_movemask_epi8(_mm_cmpeq_epi8(a, b)); }", 0),
    "low32": ("int low32(__m128i a, __m128i b)\n"
              "{ return _mm_cvtsi128_si32(_mm_add_epi32(a, b)); }", 0),
    "signs": ("int signs(__m128 a, __m128 b)\n"
              "{ return _mm_movemask_ps(_mm_cmplt_ps(a, b)); }", 0),
    "signs2": ("int signs2(__m128d a, __m128d b)\n"
               "{ return _mm_movemask_pd(_mm_cmplt_pd(a, b)) == 3; }", 0),
    "low64": ("long long low64(__m128i a, __m128i b)\n"
              "{ return _mm_cvtsi128_si64(_mm_sub_epi64(a, b)); }", 0),
    "word": ("int word(__m128i a, __m128i b)\n"
             "{ return _mm_extract_epi16(_mm_adds_epi16(a, b), 5) == 77; }", 0),
    # Two results alive at once: neither can stay in %rax.
    "both": ("int both(__m128i a, __m128i b)\n"
             "{ int x = _mm_movemask_epi8(a), y = _mm_movemask_epi8(b);\n"
             "  return x * 31 + y; }", 1),
}
# Open, recorded so that they can only get better: a 256-bit value that goes
# through a slot stays there (the store forwarder gives a 32-byte store no
# mapping), and the 256-bit float compares still compute through memory.
AVX2 = {
    # GCC leaves the upper halves to the caller that passed a 256-bit
    # argument; lccc clears them before it returns: one `vzeroupper`.
    "full32": ("int full32(__m256i a)\n"
               "{ return _mm256_movemask_epi8(a) == -1; }", 1),
}
OPEN = {
    "equal32": ("int equal32(__m256i a, __m256i b)\n"
                "{ return _mm256_movemask_epi8(_mm256_cmpeq_epi8(a, b)) == -1; }", 5),
    "signs8": ("int signs8(__m256 a, __m256 b)\n"
               "{ return _mm256_movemask_ps(_mm256_cmp_ps(a, b, 1)); }", 12),
    "signs4": ("int signs4(__m256d a)\n"
               "{ return _mm256_movemask_pd(a) != 0; }", 14),
}
AVX512 = {
    # GCC tests the mask where it is (`kortestw`); lccc moves it out and
    # compares: one instruction, the same number of micro-operations.
    "all16": ("int all16(const int *p, const int *q)\n"
              "{ __m512i a = _mm512_loadu_si512(p), b = _mm512_loadu_si512(q);\n"
              "  return _mm512_cmpge_epu32_mask(a, b) == 0xffff; }", 1),
    "none16": ("int none16(const int *p, const int *q)\n"
               "{ __m512i a = _mm512_loadu_si512(p), b = _mm512_loadu_si512(q);\n"
               "  return _mm512_cmpge_epu32_mask(a, b) == 0; }", 1),
    # GCC moves `k` into a mask register and compares under it; lccc ands
    # in a general register and copies the result to %eax.
    "under": ("__mmask16 under(__mmask16 k, const int *p, const int *q)\n"
              "{ __m512i a = _mm512_loadu_si512(p), b = _mm512_loadu_si512(q);\n"
              "  return _mm512_mask_cmplt_epi32_mask(k, a, b); }", 1),
    "count": ("int count(const int *p, const int *q)\n"
              "{ __m512i a = _mm512_loadu_si512(p), b = _mm512_loadu_si512(q);\n"
              "  return __builtin_popcount(_mm512_cmplt_epi32_mask(a, b)); }", 0),
    "bytes": ("unsigned long long bytes(const char *p, const char *q)\n"
              "{ __m512i a = _mm512_loadu_si512(p), b = _mm512_loadu_si512(q);\n"
              "  return _mm512_cmpeq_epi8_mask(a, b); }", 0),
}
GROUPS = [
    ("sse2", [], set(), SSE2),
    ("avx2", ["-mavx2"], {"avx2"}, AVX2 | OPEN),
    ("avx512", ["-mavx512f", "-mavx512bw", "-mavx512vl"],
     {"avx512f", "avx512bw", "avx512vl"}, AVX512),
]

MAIN = r"""
#include <stdio.h>
static unsigned long long state = 0x243f6a8885a308d3ull;
static unsigned long long next(void)
{ state ^= state << 13; state ^= state >> 7; state ^= state << 17; return state; }
static void fill(unsigned char *p, int n, int round)
{
    for (int i = 0; i < n; i++) p[i] = (unsigned char)(next() >> 24);
    /* Runs of equal and of ordered data, so every mask value turns up. */
    if (round % 3 == 0) for (int i = 0; i < n; i++) p[i] = (unsigned char)(i * 7 + round);
    if (round % 5 == 0) for (int i = n / 2; i < n; i++) p[i] = 0x80;
}
int main(void)
{
    unsigned char x[64] __attribute__((aligned(64))), y[64] __attribute__((aligned(64)));
    unsigned long long sum = 0;
    for (int round = 0; round < 200; round++) {
        fill(x, 64, round);
        fill(y, 64, round + (round % 4 == 0 ? 0 : 1));
        if (round % 7 == 0) __builtin_memcpy(y, x, 64);
        if (round % 11 == 0) y[round % 64] ^= 1;
@CALLS@
    }
    printf("%llu\n", sum);
    return 0;
}
"""
CALLS = {
    "sse2": [
        "{ __m128i a = _mm_load_si128((void *)x), b = _mm_load_si128((void *)y);",
        "  __m128 f = _mm_cvtepi32_ps(a), g = _mm_cvtepi32_ps(b);",
        "  __m128d d = _mm_cvtepi32_pd(a), e = _mm_cvtepi32_pd(b);",
        "  sum = sum * 1099511628211ull + (unsigned)equal16(a, b);",
        "  sum = sum * 1099511628211ull + (unsigned)mask16(a, b);",
        "  sum = sum * 1099511628211ull + (unsigned)low32(a, b);",
        "  sum = sum * 1099511628211ull + (unsigned)signs(f, g);",
        "  sum = sum * 1099511628211ull + (unsigned)signs2(d, e);",
        "  sum = sum * 1099511628211ull + (unsigned long long)low64(a, b);",
        "  sum = sum * 1099511628211ull + (unsigned)word(a, b);",
        "  sum = sum * 1099511628211ull + (unsigned)both(a, b); }",
    ],
    "avx2": [
        "{ __m256i a = _mm256_load_si256((void *)x), b = _mm256_load_si256((void *)y);",
        "  __m256 f = _mm256_cvtepi32_ps(a), g = _mm256_cvtepi32_ps(b);",
        "  sum = sum * 1099511628211ull + (unsigned)full32(a);",
        "  sum = sum * 1099511628211ull + (unsigned)signs4(_mm256_castsi256_pd(b));",
        "  sum = sum * 1099511628211ull + (unsigned)equal32(a, b);",
        "  sum = sum * 1099511628211ull + (unsigned)signs8(f, g); }",
    ],
    "avx512": [
        "  sum = sum * 1099511628211ull + (unsigned)all16((void *)x, (void *)y);",
        "  sum = sum * 1099511628211ull + (unsigned)none16((void *)x, (void *)y);",
        "  sum = sum * 1099511628211ull + (unsigned)under((unsigned short)(round * 2654435761u),"
        " (void *)x, (void *)y);",
        "  sum = sum * 1099511628211ull + (unsigned)count((void *)x, (void *)y);",
        "  sum = sum * 1099511628211ull + bytes((void *)x, (void *)y);",
    ],
}


def problem(text: str) -> None:
    PROBLEMS.append(text)


def run(command: list[str], **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(command, capture_output=True, text=True, timeout=300, **kwargs)


def cpu_flags() -> set[str]:
    try:
        with open("/proc/cpuinfo", encoding="utf-8") as handle:
            for line in handle:
                if line.startswith("flags"):
                    return set(line.split(":", 1)[1].split())
    except OSError:
        pass
    return set()


def functions(assembly: str) -> dict[str, list[str]]:
    """The instructions of each function, by name."""
    out: dict[str, list[str]] = {}
    current = None
    for raw in assembly.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        label = re.fullmatch(r"([A-Za-z_][A-Za-z0-9_]*):", line)
        if label:
            current = label.group(1)
            out[current] = []
        elif line.startswith(".") or line.endswith(":"):
            continue
        elif current is not None:
            out[current].append(re.sub(r"\s+", " ", line))
    return out


def source(group: dict[str, tuple[str, int]], attribute: str = "__attribute__((noinline)) ") -> str:
    return "#include <immintrin.h>\n" + "".join(attribute + body + "\n" for body, _ in group.values())


def shapes(name: str, group: dict[str, tuple[str, int]], lccc: list[str], gcc: list[str] | None,
           directory: str) -> None:
    path = os.path.join(directory, f"{name}.c")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(source(group))
    built = run(lccc + ["-S", "-o", "-", path])
    if built.returncode != 0:
        problem(f"{name}: lccc does not compile the probe: {built.stderr.strip()[:300]}")
        return
    ours = functions(built.stdout)
    theirs = None
    if gcc is not None:
        reference = run(gcc + ["-S", "-o", "-", path])
        if reference.returncode != 0:
            problem(f"{name}: GCC does not compile the probe: {reference.stderr.strip()[:300]}")
        else:
            theirs = functions(reference.stdout)
    for function, (_, slack) in group.items():
        body = ours.get(function)
        if not body:
            problem(f"{name}/{function}: no such function in lccc's output")
            continue
        text = "\n    ".join(body)
        if function in OPEN:
            if theirs is not None and theirs.get(function):
                if len(body) > len(theirs[function]) + slack:
                    problem(f"{name}/{function}: {len(body)} instructions, GCC"
                            f" {len(theirs[function])}; recorded open at"
                            f" {len(theirs[function]) + slack}, and it got longer:\n    {text}")
                elif len(body) < len(theirs[function]) + slack:
                    problem(f"{name}/{function}: {len(body)} instructions where"
                            f" {len(theirs[function]) + slack} are recorded open:"
                            f" it got better, lower the record")
            continue
        for line in body:
            if STACK.search(line):
                problem(f"{name}/{function}: touches the stack (`{line}`):\n    {text}")
                break
        for line in body:
            if VECTOR_COPY.match(line):
                problem(f"{name}/{function}: copies a vector register (`{line}`):\n    {text}")
                break
        for line in body:
            if line.startswith("vpcmp") and "%zmm1," in line + ",":
                problem(f"{name}/{function}: the compare reads a loaded copy (`{line}`):\n    {text}")
        if theirs is not None:
            expected = theirs.get(function)
            if not expected:
                problem(f"{name}/{function}: no such function in GCC's output")
            elif len(body) > len(expected) + slack:
                problem(f"{name}/{function}: {len(body)} instructions, GCC {len(expected)}"
                        f" (allowed {len(expected) + slack}):\n    {text}")


def values(name: str, group: dict[str, tuple[str, int]], lccc: list[str], gcc: list[str],
           directory: str) -> int:
    """Build the probe and its driver with each compiler; the sums must agree."""
    path = os.path.join(directory, f"{name}_run.c")
    with open(path, "w", encoding="utf-8") as handle:
        calls = "\n".join("        " + line for line in CALLS[name])
        handle.write(source(group) + MAIN.replace("@CALLS@", calls))
    reference = None
    runs = 0
    for label, compiler, levels in (("gcc", gcc, ["-O2"]), ("lccc", lccc, ["-O0", "-O1", "-O2", "-O3"])):
        for level in levels:
            binary = os.path.join(directory, f"{name}.{label}{level}")
            built = run([compiler[0]] + [level] + compiler[2:] + ["-o", binary, path])
            if built.returncode != 0:
                problem(f"{name}: {label} {level} does not build the driver:"
                        f" {built.stderr.strip()[:300]}")
                continue
            ran = run([binary])
            runs += 1
            if ran.returncode != 0:
                problem(f"{name}: {label} {level} exits with {ran.returncode}")
            elif reference is None:
                reference = ran.stdout
            elif ran.stdout != reference:
                problem(f"{name}: {label} {level} prints {ran.stdout.strip()},"
                        f" GCC {reference.strip()}")
    return runs


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawTextHelpFormatter)
    parser.add_argument("--lccc", required=True)
    parser.add_argument("--gcc")
    parser.add_argument("--cpu-flags", metavar="FLAGS",
                        help="what the CPU has, in place of /proc/cpuinfo (for testing the gate)")
    arguments = parser.parse_args()
    cpu = cpu_flags() if arguments.cpu_flags is None else set(arguments.cpu_flags.split())
    checked = 0
    runs = 0
    notes = []
    with tempfile.TemporaryDirectory(prefix="lccc-simd-int-") as directory:
        for name, options, needs, group in GROUPS:
            lccc = [arguments.lccc, "-O2"] + options
            gcc = [arguments.gcc, "-O2"] + options if arguments.gcc else None
            shapes(name, group, lccc, gcc, directory)
            checked += len(group)
            if gcc is None:
                continue
            if needs <= cpu:
                runs += values(name, group, lccc, gcc, directory)
            else:
                missing = " ".join(sorted(needs - cpu))
                notes.append(f"{name} built and not run (the CPU has no {missing})")
    if PROBLEMS:
        for text in PROBLEMS:
            print(f"FAIL: {text}")
        print(f"check_x86_simd_integer_results: {len(PROBLEMS)} problem(s)")
        return 1
    against = "; lengths against GCC's" if arguments.gcc else "; no GCC given, lengths not compared"
    print(f"PASS: {checked - len(OPEN)} functions keep their integer results in registers"
          f"{against}; {len(OPEN)} recorded open ({', '.join(OPEN)});"
          f" {runs} runs equal to GCC's" + "".join(f"; {note}" for note in notes))
    return 0


if __name__ == "__main__":
    sys.exit(main())
