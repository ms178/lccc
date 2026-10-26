#!/usr/bin/env python3
"""Derive and verify the argument count of every builtin intrinsic.

AUD-ICE-1: `_mm_add_ps()` -- or any other intrinsic called with the wrong
number of arguments -- used to reach the backend's operand indexing and panic
(`ccc: internal error: emit_sse_binary_128: malformed intrinsic addps (0 args)`).
GCC answers "too few arguments to function '_mm_add_ps'" instead, and so must a
compiler that claims the same command line.

The front end cannot ask a prototype: an intrinsic is only a prototype when the
requesting translation unit included the header that declares it, and the
bundled headers declare the `_mm_*`/`_mm256_*` wrappers -- not the
`__builtin_ia32_*` primitives they are built from.  What the headers *do*
provide is the arity itself, twice over:

  * `__builtin_ia32_*` appear as call sites inside the wrapper bodies
    (`return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_paddb128(__a, __b));`),
  * every `_mm*` wrapper appears as a declaration whose parameter list is the
    wrapper's arity (`static __forceinline __m128 _mm_add_ps(__m128 a, __m128 b)`).

`src/frontend/sema/intrinsic_arity.rs` is this derivation, checked in.  This
script is the derivation and the gate that keeps the two in step: run it after
touching `include/*.h`, and CI runs `--check` so the checked-in table cannot
rot.  A name whose header uses disagree about its arity (or that no header
mentions at all) is left out of the table: the front end then accepts whatever
it accepted before, which is the only safe default for a guess.

Usage:
    scripts/check_intrinsic_arity.py --check
    scripts/check_intrinsic_arity.py --write
    scripts/check_intrinsic_arity.py --self-test
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

RUST_TABLE = "src/frontend/sema/intrinsic_arity.rs"
HEADER_GLOB = "include/*.h"
NAME_TABLE = "src/frontend/sema/builtins.rs"

ENTRY = re.compile(
    r'm\.insert\(\s*"([^"]+)"\s*,\s*BuiltinInfo::intrinsic\(\s*(?:BuiltinIntrinsic::)?(\w+)\s*\)',
    re.S,
)
BUILTIN_CALL = re.compile(r"\b((?:__builtin_ia32_|__lccc_simd(?:128|256|512)_(?:i|ps|pd)_)\w+)\s*\(")
HELPER_DECL = re.compile(r"^[^\n;#={}]*?\b(__lccc_simd(?:128|256|512)_(?:i|ps|pd)_\w+)\s*\(", re.M)
# A `_mm*` wrapper declaration or definition: the name is preceded on its line
# by nothing but a return type, and is not the callee of an assignment.
WRAPPER_MACRO = re.compile(r"^\s*#\s*define\s+(_mm\w*)\(", re.M)
WRAPPER_DECL = re.compile(r"^[^\n;#=]*?\b(_mm[\w]*)\s*\(", re.M)
COMMENT = re.compile(r"/\*.*?\*/|//[^\n]*", re.S)


def found_repo() -> Path:
    cand = os.environ.get("LCCC_REPO")
    if cand and (Path(cand) / NAME_TABLE).exists():
        return Path(cand)
    here = Path.cwd().resolve()
    for c in (here, *here.parents):
        if (c / NAME_TABLE).exists():
            return c
    return Path(__file__).resolve().parent.parent


def count_args(text: str, open_paren: int) -> int:
    """Number of top-level arguments in the call whose '(' is at `open_paren`."""
    depth = 1
    commas = 0
    seen = False
    i = open_paren + 1
    while i < len(text) and depth:
        c = text[i]
        if c in "\"'":
            # Skip literals: a comma inside "a,b" is not an argument separator.
            quote = c
            i += 1
            while i < len(text) and text[i] != quote:
                i += 2 if text[i] == "\\" else 1
            i += 1
            continue
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
            if depth == 0:
                break
        elif c == "," and depth == 1:
            commas += 1
        elif not c.isspace():
            seen = True
        i += 1
    if text[open_paren + 1:i].strip() == "void":
        return 0  # A C nullary prototype, not one operand.
    return commas + 1 if seen else 0


def intrinsic_names(repo: Path) -> set[str]:
    names = {n for n, _k in ENTRY.findall((repo / NAME_TABLE).read_text())}
    for module in ("avx_test", "avx_widen"):
        path = repo / f"src/common/{module}.rs"
        if path.exists():
            names.update(re.findall(r'"((?:_mm|__builtin_ia32_|__lccc_)\w*)"', path.read_text()))
    return names


# Primitives the bundled headers never mention, so no derivation can see
# them -- but whose arity is not a guess: each entry is GCC\'s own builtin
# signature (the oracle the headers target; `__builtin_ia32_loaddqu` is
# "expected 1" in GCC 16.2\'s diagnostic) AND the count this compiler\'s
# lowering/backend already assume when they index the operand list.  The
# dqu load/store family is the whole list because its wrappers
# (`_mm_loadu_si128` and family) are bound straight to the intrinsic kind
# in the name table and their header bodies are aggregate dereferences, so
# no `__builtin_ia32_loaddqu(...)` call site exists anywhere to derive
# from.  An entry that contradicts a header-derived count is a hard error
# in derive(): the supplement may only fill a genuine gap.
SUPPLEMENT = {
    "__builtin_ia32_loaddqu": 1,
    "__builtin_ia32_loaddqu256": 1,
    "__builtin_ia32_storedqu": 2,
    "__builtin_ia32_storedqu256": 2,
}


def derive(repo: Path) -> dict[str, int]:
    """name -> arity, for every intrinsic whose headers agree on one answer."""
    counts: dict[str, set[int]] = {}
    helpers = set()
    for header in sorted(repo.glob(HEADER_GLOB)):
        text = COMMENT.sub(" ", header.read_text())
        for m in HELPER_DECL.finditer(text):
            helpers.add(m.group(1))
        for m in BUILTIN_CALL.finditer(text):
            counts.setdefault(m.group(1), set()).add(count_args(text, m.end() - 1))
        for m in (*WRAPPER_DECL.finditer(text), *WRAPPER_MACRO.finditer(text)):
            counts.setdefault(m.group(1), set()).add(count_args(text, m.end() - 1))
    # Canonical GCC signatures also pin raw builtins not used by a wrapper.
    # COUNT/CONVERT/PTEST are descriptor properties, not C parameters.
    facts = repo / "engineering/isa/gnu-x86-builtin-types.tsv"
    if facts.exists():
        for line in facts.read_text().splitlines():
            if not line or line.startswith("#"):
                continue
            name, descriptor, _mode = line.split("\t")
            parameters = descriptor.split("_FTYPE_", 1)[1].split("_")
            arity = len([p for p in parameters if p not in ("VOID", "COUNT", "CONVERT", "PTEST")])
            counts.setdefault(name, set()).add(arity)
    known = intrinsic_names(repo) | helpers
    table = {}
    for name, n in counts.items():
        if name in known and len(n) == 1:
            (arity,) = n
            if arity <= 8:
                table[name] = arity
    for name, arity in SUPPLEMENT.items():
        if name not in known:
            raise SystemExit(f"supplement names an unknown intrinsic: {name}")
        if name in table and table[name] != arity:
            raise SystemExit(
                f"supplement contradicts the headers for {name}: "
                f"{arity} != {table[name]}"
            )
        table[name] = arity
    return table


def render(table: dict[str, int]) -> str:
    rows = "\n".join(f'    ("{n}", {a}),' for n, a in sorted(table.items()))
    return f'''//! Argument counts of the builtin intrinsics, derived from `include/*.h`.
//!
//! AUD-ICE-1: an intrinsic called with the wrong number of arguments used to
//! reach the backend's operand indexing and panic.  `_mm_add_ps()` is not a
//! prototype violation a translation unit can be blamed for when it never
//! included the header -- and the `__builtin_ia32_*` primitives are never
//! declared anywhere -- so the count lives next to the name table instead.
//!
//! Generated by `scripts/check_intrinsic_arity.py --write`, which reads the
//! arity out of the bundled headers themselves: the `__builtin_ia32_*`
//! primitives and `__lccc_simd*` helpers from call sites/prototypes, `_mm_*`
//! wrappers from their own parameter lists, plus a four-entry supplement
//! (the `loaddqu`/`storedqu` primitives no header mentions; each count is
//! GCC's builtin signature and the count this compiler's backend assumes).
//! A name the headers disagree about, or never mention, is absent here and
//! unconstrained in the front end.
//! `scripts/check_intrinsic_arity.py --check` (wired into CI) re-derives the
//! table and fails if it stops matching, so this file cannot rot.

/// Argument count of `name`, or `None` when the headers do not pin one down.
pub(crate) fn builtin_intrinsic_arity(name: &str) -> Option<usize> {{
    ARITY_TABLE
        .binary_search_by_key(&name, |row| row.0)
        .ok()
        .map(|i| ARITY_TABLE[i].1 as usize)
}}

/// Sorted by name, which `builtin_intrinsic_arity` relies on.
const ARITY_TABLE: &[(&str, u8)] = &[
{rows}
];

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn table_is_sorted_and_binary_search_reaches_every_row() {{
        for pair in ARITY_TABLE.windows(2) {{
            assert!(
                pair[0].0 < pair[1].0,
                "unsorted: {{}} {{}}",
                pair[0].0,
                pair[1].0
            );
        }}
        for (name, arity) in ARITY_TABLE {{
            assert_eq!(
                builtin_intrinsic_arity(name),
                Some(*arity as usize),
                "{{name}} not reachable"
            );
        }}
    }}

    #[test]
    fn known_wrappers_and_primitives_carry_their_header_arity() {{
        // Wrapper declarations from `include/emmintrin.h`: two operands.
        assert_eq!(builtin_intrinsic_arity("_mm_add_epi8"), Some(2));
        assert_eq!(builtin_intrinsic_arity("_mm_cmpeq_epi8"), Some(2));
        // Primitive call sites, same header.
        assert_eq!(builtin_intrinsic_arity("__builtin_ia32_paddb128"), Some(2));
        // A nullary wrapper must stay nullary, not absent: the difference is
        // the difference between diagnosing `_mm256_zeroupper(1)` and
        // panicking on it.
        if let Some(arity) = builtin_intrinsic_arity("_mm256_zeroupper") {{
            assert_eq!(arity, 0);
        }}
        assert_eq!(builtin_intrinsic_arity("__builtin_ia32_clflush"), Some(1));
    }}

    #[test]
    fn unknown_and_ambiguous_names_are_unconstrained() {{
        assert_eq!(builtin_intrinsic_arity("__builtin_not_a_builtin"), None);
        assert_eq!(builtin_intrinsic_arity("_mm_not_a_wrapper"), None);
        // Variadic fortify primitives must never be arity-checked.
        assert_eq!(builtin_intrinsic_arity("__builtin___snprintf_chk"), None);
    }}
}}
'''


def parse_rust_table(repo: Path) -> dict[str, int]:
    text = (repo / RUST_TABLE).read_text()
    return {n: int(a) for n, a in re.findall(r'\("([^"]+)", (\d+)\),', text)}


def self_test(repo: Path) -> int:
    assert count_args("f()", 1) == 0
    assert count_args("f(void)", 1) == 0
    assert count_args("f(void *)", 1) == 1
    assert count_args("f(a)", 1) == 1
    assert count_args("f(a, b)", 1) == 2
    assert count_args("f((a, b), c)", 1) == 2
    assert count_args("f(g(1, 2), h(3))", 1) == 2
    assert count_args('f("a,b", c)', 1) == 2
    assert count_args("f(((int (*)(int, int))0), b)", 1) == 2
    assert BUILTIN_CALL.search("__builtin_ia32_paddb128(__a, __b)")
    assert WRAPPER_DECL.search("static __inline __m128 _mm_add_ps(__m128 a, __m128 b) {")
    assert not WRAPPER_DECL.search("x = _mm_add_ps(a, b);")
    # The derivation must actually see both families in the shipped headers.
    table = derive(repo)
    assert table.get("__builtin_ia32_paddb128") == 2, table.get("__builtin_ia32_paddb128")
    assert table.get("_mm_add_ps") == 2, table.get("_mm_add_ps")
    assert table.get("__lccc_simd128_ps_divps128") == 3
    assert table.get("_mm_setzero_ps") == 0
    assert len(table) > 800, len(table)
    print(f"self-test: derivation sees {len(table)} intrinsics in {HEADER_GLOB}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--write", action="store_true", help="regenerate the Rust table")
    ap.add_argument("--check", action="store_true", help="fail if the table is stale")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if not (args.write or args.check or args.self_test):
        ap.error("pick one of --write, --check, --self-test")

    repo = found_repo()
    if args.self_test:
        return self_test(repo)

    derived = derive(repo)
    if args.write:
        (repo / RUST_TABLE).write_text(render(derived))
        print(f"wrote {len(derived)} arities to {RUST_TABLE}")
        return 0

    checked_in = parse_rust_table(repo)
    missing = {n: a for n, a in derived.items() if checked_in.get(n) != a}
    stale = {n: a for n, a in checked_in.items() if derived.get(n) != a}
    if missing or stale:
        print(
            "FAIL: the checked-in arity table disagrees with include/*.h "
            f"({len(missing)} missing/changed, {len(stale)} no longer derivable)",
            file=sys.stderr,
        )
        for n, a in sorted(missing.items())[:10]:
            print(f"  headers say {n}({a} args); table says {checked_in.get(n)}", file=sys.stderr)
        for n, a in sorted(stale.items())[:10]:
            print(f"  table says {n}({a} args); headers no longer say so", file=sys.stderr)
        print("run scripts/check_intrinsic_arity.py --write", file=sys.stderr)
        return 1
    print(
        f"arity table matches the headers: {len(checked_in)} intrinsics pinned "
        f"across {len(sorted(repo.glob(HEADER_GLOB)))} headers"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
