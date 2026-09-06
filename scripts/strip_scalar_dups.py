#!/usr/bin/env python3
"""Remove from the legacy intrinsic headers everything that lcccsimd.h now
defines (vector typedefs + function wrappers + same-name macros), so that
including both cannot cause redefinition errors.

Brace-matched forward-scan function removal. Conservative: only removes a
function when its parsed declaration name is exactly the requested name.

Usage: python3 scripts/strip_scalar_dups.py
"""
import re
from pathlib import Path

REPO = Path(__file__).parent.parent
INC = REPO / "include"
SIMD = (INC / "lcccsimd.h").read_text()

LEGACY = ["xmmintrin.h", "emmintrin.h", "pmmintrin.h", "smmintrin.h",
          "tmmintrin.h", "nmmintrin.h", "avxintrin.h", "avx2intrin.h",
          "avx512fintrin.h", "fmaintrin.h", "wmmintrin.h"]

# names defined by lcccsimd.h (wrappers)
NAMES = {n for n in re.findall(r"\b(_mm(?:256|512)?[a-zA-Z0-9_]*)\s*\(", SIMD)
         if not n.startswith("_mmask")}

# vector typedefs to remove from legacy headers
TYPE_TYS = {
    "emmintrin.h": ["__m128i", "__m128i_u", "__m128d", "__m128d_u"],
    "xmmintrin.h": ["__m128", "__m128_u"],
    "avxintrin.h": ["__m256", "__m256d", "__m256i", "__m256_u", "__m256d_u", "__m256i_u"],
    "avx512fintrin.h": ["__m512i", "__m512d", "__m512", "__m512i_u",
                        "__mmask8", "__mmask16", "__mmask32", "__mmask64"],
}


def remove_typedef(text, ty):
    pat = re.compile(
        r"typedef\s+struct\s+__attribute__\(\(__aligned__\(\d+\)\)\)\s*\{[^}]*\}\s*"
        + re.escape(ty) + r"\s*;\s*", re.S)
    return pat.sub("", text)


def remove_vvector_typedefs(text):
    pat = re.compile(
        r"typedef\s+[a-z_]+\s+__v[a-z0-9]+\s+__attribute__\s*\(\(\s*__vector_size__\s*\(\d+\)\s*\)\)\s*;\s*",
        re.S)
    return pat.sub("", text)


def remove_function(text, name):
    """Remove `static __inline__ ... name(...) { ... }` (attribute may sit on
    a line of its own between the `static __inline__` header and the name)."""
    lines = text.split("\n")
    n = len(lines)
    removed = 0
    out = []
    i = 0
    while i < n:
        line = lines[i]
        if "static __inline__" in line or "static __inline" in line:
            j = i
            decl_name = None
            while j < n:
                m = re.match(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(", lines[j])
                if m:
                    decl_name = m.group(1)
                    break
                j += 1
                if "{" in lines[j - 1] or j - i > 4:
                    break
            if decl_name == name and j < n:
                k = j
                depth = 0
                started = False
                end = None
                while k < n:
                    for ch in lines[k]:
                        if ch == '{':
                            depth += 1
                            started = True
                        elif ch == '}':
                            depth -= 1
                            if started and depth == 0:
                                end = k
                                break
                    if end is not None:
                        break
                    k += 1
                if end is not None:
                    removed += 1
                    i = end + 1
                    continue
        out.append(line)
        i += 1
    return "\n".join(out), removed


def remove_same_name_macros(text, names):
    lines = text.split("\n")
    out = []
    i = 0
    n = len(lines)
    removed = 0
    while i < n:
        line = lines[i]
        m = re.match(r"\s*#\s*define\s+(_mm(?:256|512)?[a-zA-Z0-9_]*)\s*\(", line)
        if m and m.group(1) in names:
            while i < n and lines[i].rstrip().endswith("\\"):
                i += 1
            i += 1
            removed += 1
            continue
        out.append(line)
        i += 1
    return "\n".join(out), removed


def main():
    total = 0
    for hname in LEGACY:
        p = INC / hname
        text = p.read_text()
        for ty in TYPE_TYS.get(hname, []):
            text = remove_typedef(text, ty)
        text = remove_vvector_typedefs(text)
        text, r = remove_same_name_macros(text, NAMES)
        if r:
            print(f"{hname}: removed {r} same-name macros")
        total += r
        removed = 0
        for name in sorted(NAMES, key=len, reverse=True):
            text, r = remove_function(text, name)
            removed += r
        p.write_text(text)
        total += removed
        print(f"{hname}: removed {removed} function definitions")
    print(f"total removed: {total}")


if __name__ == "__main__":
    main()
