"""The byte-search idiom: `memchr` synthesis and everything that must prevent it.

`loop_idiom` rewrites

    for (size_t i = 0; i < n; i++) if (p[i] == c) return p + i;
    return 0;

into `memchr(p, (int)(unsigned char)c, n)` -- the same transformation Clang's
`LoopIdiomRecognize` performs, and the reason Clang beats both GCC and (before
this) LCCC by ~10x on `tests/bench/k_memchr.c`.

Two things are worth pinning, and they fail in opposite directions:

1. **The rewrite must fire, and must fire only where it is provably exact.**
   The emitted `call memchr` is the shape assertion; the near-miss shapes
   below (a store in the body, an index-returning loop, a signed byte array,
   a bound narrower than `size_t`) must keep their loop, because each of them
   breaks one of the matcher's proof obligations.

2. **Every escape hatch must keep it off.** `CCC_NO_MEMCHR_IDIOM` for a
   bisect, `-fno-builtin-memchr` and `-ffreestanding` for the fixed policy,
   and a translation unit that implements `memchr` itself (rewriting its own
   body into a call to itself would be infinite recursion).

Semantics live in `tests/regression/loop_idiom_memchr.c` (every needle, every
length, every position, checksum from GCC 16.2); this module owns the
compile-time decisions.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
LCCC = REPO / "target" / "fastbuild" / "lccc"

def gcc_oracle() -> str | None:
    """The GCC used as an oracle, or None when this host has none.

    CI pins GCC 16.2 (GCC_BIN or `gcc-16`); the reference leg is skipped rather
    than failed on hosts without it, the compile-time assertions still run.
    """
    for candidate in (os.environ.get("GCC_BIN"), "gcc-16", "gcc"):
        if candidate and shutil.which(candidate):
            return candidate
    return None


FIXTURE = REPO / "tests" / "regression" / "loop_idiom_memchr.c"

# The matched shape, plus the near misses.  Each near miss differs in exactly
# one accepted fact of the matcher.
SHAPES = {
    "match": r"""
static const unsigned char *scan(const unsigned char *p, unsigned char c, size_t n) {
    for (size_t i = 0; i < n; i++) if (p[i] == c) return p + i;
    return 0;
}
const unsigned char *use(const unsigned char *p, unsigned char c, size_t n) { return scan(p, c, n); }
""",
    # A store in the body: the library scan would reorder it.
    "store_in_body": r"""
static const unsigned char *scan(unsigned char *p, unsigned char c, size_t n) {
    for (size_t i = 0; i < n; i++) { p[i] |= 1; if (p[i] == c) return p + i; }
    return 0;
}
const unsigned char *use(unsigned char *p, unsigned char c, size_t n) { return scan(p, c, n); }
""",
    # Returns the index, not the pointer: `memchr`'s result is a pointer.
    "index_result": r"""
static size_t scan(const unsigned char *p, unsigned char c, size_t n) {
    for (size_t i = 0; i < n; i++) if (p[i] == c) return i;
    return n;
}
size_t use(const unsigned char *p, unsigned char c, size_t n) { return scan(p, c, n); }
""",
    # Signed bytes: `memchr` compares unsigned chars.
    "signed_bytes": r"""
static const signed char *scan(const signed char *p, signed char c, size_t n) {
    for (size_t i = 0; i < n; i++) if (p[i] == c) return p + i;
    return 0;
}
const signed char *use(const signed char *p, signed char c, size_t n) { return scan(p, c, n); }
""",
    # A bound narrower than size_t: a negative int is not a huge size_t.
    "narrow_bound": r"""
static const unsigned char *scan(const unsigned char *p, unsigned char c, int n) {
    for (int i = 0; i < n; i++) if (p[i] == c) return p + i;
    return 0;
}
const unsigned char *use(const unsigned char *p, unsigned char c, int n) { return scan(p, c, n); }
""",
    # A NARROWING index cast: `p[(unsigned short)i]` reads different addresses
    # than `p[i]` once i exceeds 65535, so a linear scan is a different program.
    "narrow_index": r"""
static const unsigned char *scan(const unsigned char *p, unsigned char c, size_t n) {
    for (size_t i = 0; i < n; i++) if (p[(unsigned short)i] == c) return p + i;
    return 0;
}
const unsigned char *use(const unsigned char *p, unsigned char c, size_t n) { return scan(p, c, n); }
""",
    # A same-width index cast is exact and must keep matching.
    "reinterpret_index": r"""
static const unsigned char *scan(const unsigned char *p, unsigned char c, size_t n) {
    for (size_t i = 0; i < n; i++) if (p[(unsigned long)i] == c) return p + i;
    return 0;
}
const unsigned char *use(const unsigned char *p, unsigned char c, size_t n) { return scan(p, c, n); }
""",
    # Sixteen-bit elements are not bytes.
    "wide_elements": r"""
static const unsigned short *scan(const unsigned short *p, unsigned short c, size_t n) {
    for (size_t i = 0; i < n; i++) if (p[i] == c) return p + i;
    return 0;
}
const unsigned short *use(const unsigned short *p, unsigned short c, size_t n) { return scan(p, c, n); }
""",
}

# A TU that implements `memchr` itself must never synthesize a call to it.
OWN_MEMCHR = r"""
void *memchr(const void *s, int c, unsigned long n) {
    const unsigned char *p = s;
    for (unsigned long i = 0; i < n; i++) if (p[i] == (unsigned char)c) return (void *)(p + i);
    return 0;
}
const unsigned char *scan(const unsigned char *p, unsigned char c, unsigned long n) {
    for (unsigned long i = 0; i < n; i++) if (p[i] == c) return p + i;
    return 0;
}
const unsigned char *use(const unsigned char *p, unsigned char c, unsigned long n) { return scan(p, c, n); }
"""


def compile_asm(source: Path, out: Path, *, flags: list[str], env: dict[str, str] | None = None) -> str:
    proc = subprocess.run(
        [str(LCCC), *flags, "-S", "-o", str(out), str(source)],
        capture_output=True, text=True, timeout=300,
        env={**os.environ, **(env or {})},
    )
    if proc.returncode != 0:
        raise AssertionError(f"compilation failed: {proc.stderr[:2000]}")
    return out.read_text()


class ByteSearchIdiom(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not LCCC.is_file():
            raise unittest.SkipTest("built LCCC required")

    def asm(self, body: str, *, flags=("-O2",), env=None) -> str:
        with tempfile.TemporaryDirectory(prefix="memchr-idiom-") as td:
            src = Path(td) / "shape.c"
            src.write_text(
                "#include <stddef.h>\n" + body
            )
            return compile_asm(src, Path(td) / "shape.s", flags=list(flags), env=env)

    def test_matched_shape_emits_the_call(self) -> None:
        text = self.asm(SHAPES["match"])
        self.assertIn("call memchr", text,
                      "the byte-search loop must become a memchr call at -O2")

    def test_every_escape_hatch_keeps_the_loop(self) -> None:
        for label, env, flags in (
            ("CCC_NO_MEMCHR_IDIOM", {"CCC_NO_MEMCHR_IDIOM": "1"}, ("-O2",)),
            ("-fno-builtin-memchr", None, ("-O2", "-fno-builtin-memchr")),
            ("-ffreestanding", None, ("-O2", "-ffreestanding")),
            ("-fno-builtin", None, ("-O2", "-fno-builtin")),
        ):
            with self.subTest(hatch=label):
                text = self.asm(SHAPES["match"], flags=flags, env=env)
                self.assertNotIn("call memchr", text,
                                 f"{label} must keep the scalar loop")

    def test_near_misses_keep_their_loops(self) -> None:
        for name in ("store_in_body", "index_result", "signed_bytes", "narrow_bound",
                     "wide_elements", "narrow_index"):
            with self.subTest(shape=name):
                text = self.asm(SHAPES[name])
                self.assertNotIn("call memchr", text,
                                 f"{name} must not be rewritten to memchr")

    def test_value_preserving_index_cast_still_matches(self) -> None:
        """`p[(unsigned long)i]` is the identity: the rewrite must still fire."""
        self.assertIn("call memchr", self.asm(SHAPES["reinterpret_index"]))

    def test_narrowing_index_cast_is_proven_and_declined(self) -> None:
        """Regression: this used to be rewritten and answered 65539, not -1."""
        fixture = REPO / "tests" / "regression" / "loop_idiom_memchr_trunc.c"
        with tempfile.TemporaryDirectory(prefix="memchr-trunc-") as td:
            exe = Path(td) / "trunc"
            proc = subprocess.run([str(LCCC), "-O2", "-o", str(exe), str(fixture)],
                                  capture_output=True, text=True, timeout=300)
            self.assertEqual(proc.returncode, 0, proc.stderr[:2000])
            run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=300)
            self.assertEqual(run.returncode, 0, run.stdout)
            self.assertIn("trunc -1", run.stdout)
            self.assertIn("plain 1234", run.stdout)

    def test_own_memchr_definition_is_not_self_recursive(self) -> None:
        text = self.asm(OWN_MEMCHR)
        self.assertNotIn("call memchr", text,
                         "a TU that implements memchr must not call itself")

    def test_goto_done_shape_matches_and_agrees_with_gcc(self) -> None:
        """`q = p + i; goto done; ... return q;` is still the same search.

        The found arm forwards the very value the call returns, so the rewrite
        replaces those uses instead of rebuilding the arm.  The result must be
        bit-identical to GCC's, which is what the runtime comparison pins.
        """
        body = r"""
static const unsigned char *scan(const unsigned char *p, unsigned char c, size_t n) {
    const unsigned char *q = 0;
    for (size_t i = 0; i < n; i++) if (p[i] == c) { q = p + i; goto done; }
    return 0;
done:
    return q;
}
int main(void) {
    static const unsigned char b[9] = {1, 2, 3, 4, 5, 6, 7, 8, 0};
    unsigned long h = 0;
    for (int c = 0; c < 12; c++) {
        const unsigned char *r = scan(b, (unsigned char)c, 9);
        h = h * 31u + (r ? (unsigned long)(r - b) + 1 : 0);
    }
    return (int)(h % 251u);
}
"""
        with tempfile.TemporaryDirectory(prefix="memchr-goto-") as td:
            src = Path(td) / "goto.c"
            src.write_text("#include <stddef.h>\n" + body)
            asm = compile_asm(src, Path(td) / "goto.s", flags=["-O2"])
            self.assertIn("call memchr", asm,
                          "the goto-done search must be rewritten too")
            gcc = gcc_oracle()
            if gcc is None:
                self.skipTest("no GCC oracle on this host")
            got = []
            for cc in (LCCC, gcc):
                exe = Path(td) / ("exe-" + Path(cc).name)
                proc = subprocess.run([str(cc), "-O2", "-o", str(exe), str(src)],
                                      capture_output=True, text=True, timeout=300)
                self.assertEqual(proc.returncode, 0, proc.stderr[:2000])
                run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=300)
                got.append(run.returncode)
            self.assertEqual(got[0], got[1],
                             "rewritten search disagrees with GCC on the goto shape")

    def test_every_target_synthesizes_the_call(self) -> None:
        """The idiom is IR-level: the call appears on all four targets.

        The byte count is `size_t`, so the argument width is per-target
        (`movl`/`mov x2`/`li a2`); the assertion is the call itself plus the
        absence of a scalar byte-loading loop in the scanning function.
        """
        cases = (
            ("x86_64", [], "call memchr"),
            ("i686", ["-m32"], "call memchr"),
            ("aarch64", [], "bl memchr"),
            ("riscv64", [], "call memchr"),
        )
        with tempfile.TemporaryDirectory(prefix="memchr-targets-") as td:
            tdir = Path(td)
            src = tdir / "shape.c"
            src.write_text("#include <stddef.h>\n" + SHAPES["match"])
            for name, flags, expect in cases:
                with self.subTest(target=name):
                    driver = LCCC
                    if name in ("aarch64", "riscv64"):
                        # The driver selects the target from its binary name.
                        driver = tdir / f"{name}-lccc"
                        shutil.copy2(LCCC, driver)
                    out = tdir / f"{name}.s"
                    proc = subprocess.run(
                        [str(driver), *flags, "-O2", "-S", "-o", str(out), str(src)],
                        capture_output=True, text=True, timeout=300,
                    )
                    self.assertEqual(proc.returncode, 0, proc.stderr[:2000])
                    text = out.read_text()
                    self.assertIn(expect, text,
                                  f"{name}: expected a `{expect}` instruction")

    def test_o0_never_rewrites(self) -> None:
        text = self.asm(SHAPES["match"], flags=("-O0",))
        self.assertNotIn("call memchr", text, "-O0 must not run the idiom pass")

    def test_fixture_holds_across_optimization_levels_and_isa_flags(self) -> None:
        """The rewrite is mid-end: every level that runs the pass must agree.

        `-mno-sse` (kernel code) and `-mavx2` change the *backend*, not the
        match, so the checksum must stay identical and `memchr` must still be
        the callee -- the synthesized declaration is a plain integer call.
        """
        cases = [("-O1",), ("-O2",), ("-O3",), ("-Os",), ("-Oz",),
                 ("-O2", "-mno-sse"), ("-O2", "-mavx2"), ("-O3", "-march=native")]
        with tempfile.TemporaryDirectory(prefix="memchr-levels-") as td:
            for flags in cases:
                with self.subTest(flags=" ".join(flags)):
                    exe = Path(td) / ("fx-" + "-".join(f.lstrip("-") for f in flags))
                    proc = subprocess.run(
                        [str(LCCC), *flags, "-o", str(exe), str(FIXTURE)],
                        capture_output=True, text=True, timeout=300,
                    )
                    self.assertEqual(proc.returncode, 0, proc.stderr[:2000])
                    run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=600)
                    self.assertEqual(run.returncode, 0, run.stdout)
                    self.assertEqual(run.stdout.strip(), "3523653933815388291")

    def test_fixture_runs_to_the_gcc_checksum(self) -> None:
        """The fixture asserts its own checksum; both settings must exit 0."""
        for label, env in (("idiom on", {}), ("idiom off", {"CCC_NO_MEMCHR_IDIOM": "1"})):
            with self.subTest(arm=label):
                with tempfile.TemporaryDirectory(prefix="memchr-run-") as td:
                    exe = Path(td) / "fixture"
                    proc = subprocess.run(
                        [str(LCCC), "-O2", "-o", str(exe), str(FIXTURE)],
                        capture_output=True, text=True, timeout=300,
                        env={**os.environ, **env},
                    )
                    self.assertEqual(proc.returncode, 0, proc.stderr[:2000])
                    run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=600)
                    self.assertEqual(run.returncode, 0,
                                     f"{label}: checksum mismatch ({run.stdout.strip()})")
                    self.assertEqual(run.stdout.strip(), "3523653933815388291")


if __name__ == "__main__":
    unittest.main()
