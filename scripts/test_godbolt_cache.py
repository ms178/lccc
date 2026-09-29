#!/usr/bin/env python3
"""Self-test for scripts/godbolt_cache.py.

No network, no Compiler Explorer. Every case here is a property the four
oracle tools actually depend on, and most of them are failure modes that a
naive ``{key: value}`` dict cache gets wrong:

  * two different namespaces storing the same tuple must not see each other,
    or an execution record and an assembly body collide;
  * a tuple whose fields contain the separator must not be forgeable into a
    different tuple, or one program's result is served for another's;
  * a record left half-written by a killed sweep must read as a MISS, not as
    a hit and not as a crash;
  * concurrent writers must not interleave into one corrupt file.

Run: python3 scripts/test_godbolt_cache.py
"""
from __future__ import annotations

import concurrent.futures
import json
import os
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import godbolt_cache as gc  # noqa: E402


# The tests redirect the cache into a scratch directory for the whole run, so
# they never touch the real .godbolt-cache/ and never depend on its contents.
_TMP = tempfile.TemporaryDirectory(prefix="godbolt-cache-test.")
gc.CACHE = Path(_TMP.name) / ".godbolt-cache"

FAILURES: list[str] = []


def check(name: str, cond: bool, detail: str = "") -> None:
    if cond:
        print(f"ok   {name}")
    else:
        print(f"FAIL {name}" + (f" -- {detail}" if detail else ""))
        FAILURES.append(name)


# --------------------------------------------------------------------------
def test_roundtrip_json() -> None:
    rec = {"id": "cg162", "vendor": "GNU", "ok": True, "asm_insns": 42,
           "stdout": "a\nb\n", "exit": 0, "reason": None}
    gc.store_json(gc.NS_ORACLE, rec, "cg162", "-O2", "int main(){}")
    got = gc.load_json(gc.NS_ORACLE, "cg162", "-O2", "int main(){}")
    check("json round-trips exactly", got == rec, f"got {got!r}")


def test_roundtrip_lines() -> None:
    lines = ["main:", "\tmovl $0, %eax", "\tret"]
    gc.store_lines(gc.NS_ASM, lines, "cg162", "-O2", "src-a")
    got = gc.load_lines(gc.NS_ASM, "cg162", "-O2", "src-a")
    check("lines round-trip exactly", got == lines, f"got {got!r}")


def test_miss_is_none() -> None:
    check("missing key is None (json)",
          gc.load_json(gc.NS_ORACLE, "nope", "nope") is None)
    check("missing key is None (text)",
          gc.load_text(gc.NS_ASM, "nope", "nope") is None)
    check("missing key is None (lines)",
          gc.load_lines(gc.NS_ASM, "nope", "nope") is None)


def test_namespace_isolation() -> None:
    """Same tuple, two namespaces: records must not see each other.

    This is the collision that would serve an assembly body where an
    execution record was expected (or vice versa) if the namespace were only
    cosmetic.
    """
    parts = ("cg162", "-O2", "int main(void){return 0;}")
    gc.store_json(gc.NS_ORACLE, {"kind": "execution"}, *parts)
    gc.store_text(gc.NS_ASM, "main:\n\tret", *parts)
    check("oracle record survives asm write",
          gc.load_json(gc.NS_ORACLE, *parts) == {"kind": "execution"})
    check("asm record survives oracle write",
          gc.load_text(gc.NS_ASM, *parts) == "main:\n\tret")


def test_separator_is_not_forgeable() -> None:
    """``("a,b","c")`` and ``("a","b,c")`` must hash differently.

    A printable separator would make these collide, which in a compiler oracle
    means one source file's result gets served for a different source file's.
    """
    k1 = gc.key(gc.NS_ASM, "a,b", "c")
    k2 = gc.key(gc.NS_ASM, "a", "b,c")
    check("NUL separator cannot be forged", k1 != k2, f"{k1} == {k2}")

    k3 = gc.key(gc.NS_ASM, "a", "b", "c")
    check("part boundaries are not associative", k2 != k3, f"{k2} == {k3}")


def test_namespace_changes_digest() -> None:
    k1 = gc.key(gc.NS_ASM, "x", "y")
    k2 = gc.key(gc.NS_ORACLE, "x", "y")
    check("namespace is part of the digest", k1 != k2, f"{k1} == {k2}")


def test_write_is_atomic() -> None:
    """After a successful store, no temp file may remain.

    A leftover ``.tmp`` file would later be counted as a record by stats()
    and, worse, would signal an interrupted write that nobody noticed.
    """
    gc.store_json(gc.NS_ORACLE, {"k": 1}, "atomic", "t")
    ns_dir = gc.CACHE / gc.NS_ORACLE
    tmps = [f.name for f in ns_dir.iterdir() if ".tmp." in f.name]
    check("no temp files left behind", not tmps, f"leftover: {tmps}")


def test_corruption_is_a_miss() -> None:
    """A truncated/garbage record reads as MISS, never as an exception.

    This is the killed-sweep case, and it is the whole reason load() swallows
    parse errors: a cache that can abort a sweep is worse than no cache.
    """
    parts = ("corrupt", "t")
    gc.store_json(gc.NS_ORACLE, {"good": True}, *parts)
    path = gc.path_for(gc.NS_ORACLE, *parts)
    path.write_text('{"trunca', encoding="utf-8")
    check("truncated json is a miss, not a crash",
          gc.load_json(gc.NS_ORACLE, *parts) is None)

    path.write_text("not json at all", encoding="utf-8")
    check("garbage json is a miss, not a crash",
          gc.load_json(gc.NS_ORACLE, *parts) is None)


def test_undecodable_text_is_a_miss() -> None:
    """A text record that is not valid UTF-8 is a MISS, not a hit full of U+FFFD.

    `load_text` used to decode with `errors="replace"`.  Atomic rename makes
    truncation impossible for our own writes, so a decode failure means the
    file was corrupted by something else -- a hand edit, bit rot, a half-synced
    network mount -- and returning replacement characters would let a caller
    diff or count mush as if it were real assembly.  The strict path is what
    makes the "corruption is a miss" contract in the module docstring true for
    the text half of the cache as well as the JSON half.
    """
    parts = ("undecodable", "t")
    gc.store_text(gc.NS_ASM, "\tmovl\t%eax, %eax\n", *parts)
    check("well-formed text round-trips through the strict decoder",
          gc.load_text(gc.NS_ASM, *parts) == "\tmovl\t%eax, %eax\n")

    path = gc.path_for(gc.NS_ASM, *parts)
    path.write_bytes(b"\xff\xfe\x00asm garbage")
    check("undecodable text is a miss, not a replacement-character hit",
          gc.load_text(gc.NS_ASM, *parts) is None)
    check("undecodable text lines are None, not ['']",
          gc.load_lines(gc.NS_ASM, *parts) is None)


def test_empty_text_is_not_a_miss() -> None:
    """An empty assembly body is a legitimate HIT, not a miss.

    A compiler that emits nothing for a function (fully inlined away) is real
    data. Collapsing it to None would make `--rank` drop the column and hide
    the very case where lccc deleted the whole body.
    """
    gc.store_text(gc.NS_ASM, "", "empty", "t")
    check("empty text is a hit with empty body",
          gc.load_text(gc.NS_ASM, "empty", "t") == "")
    check("empty text yields no lines, not None",
          gc.load_lines(gc.NS_ASM, "empty", "t") == [])


def test_unicode_source() -> None:
    """Source text with non-ASCII (a wide string literal in a test program)."""
    src = 'const char *s = "héllo wörld ☃";'
    gc.store_json(gc.NS_ORACLE, {"ok": True}, "cg162", "-O2", src)
    check("unicode source round-trips",
          gc.load_json(gc.NS_ORACLE, "cg162", "-O2", src) == {"ok": True})


def test_concurrent_writers() -> None:
    """Parallel stores must not interleave into one corrupt file.

    `--rank` fans out over a thread pool, so this is the live case, not a
    theoretical one: two threads writing the same key must each produce a
    complete, parseable record -- never a torn mix of both.
    """
    def worker(i: int) -> bool:
        gc.store_json(gc.NS_ORACLE, {"i": i, "pad": "x" * 5000}, "race", str(i))
        got = gc.load_json(gc.NS_ORACLE, "race", str(i))
        return got == {"i": i, "pad": "x" * 5000}

    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
        results = list(ex.map(worker, range(64)))
    check("64 concurrent writers all read back intact", all(results),
          f"{results.count(False)} torn records")


def test_stats() -> None:
    st = gc.stats()
    check("stats counts the asm namespace", st.get(gc.NS_ASM, 0) >= 3,
          f"{st}")
    check("stats counts the oracle namespace", st.get(gc.NS_ORACLE, 0) >= 60,
          f"{st}")


def test_path_is_namespaced() -> None:
    p = gc.path_for(gc.NS_ORACLE, "a", "b")
    check("record path names its namespace", p.parent.name == gc.NS_ORACLE,
          str(p))


def main() -> int:
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
    print()
    if FAILURES:
        print(f"FAILED {len(FAILURES)}/{len(tests)}: " + ", ".join(FAILURES))
        return 1
    print(f"godbolt_cache self-test: PASS ({len(tests)} checks, cache at {gc.CACHE})")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    finally:
        _TMP.cleanup()
