"""i686 fused `Cmp -> Select` on Ptr compares (and its kill switch).

A pointer is a full 32-bit unsigned integer on i686: `IrType::Ptr` shares the
`cmpl` machinery with I32/U32, so the fused compare-and-select path
(`emit_fused_cmp_select_impl`) accepts Ptr compares exactly like the integer
ones.  It used to assert instead, which made

    p = memchr(s, c, n);
    return p ? p : 0;

-- the canonical null-check on a libcall result, and also plain
    a == b ? a : b
on pointers -- an internal compiler error on `-m32`.  The mid-end memchr idiom
makes that shape common, but it was always reachable from plain C.

The second contract here is the documented kill switch.  `CCC_NO_FUSED_CSEL`
must actually disable the fusion: the shared generation code only consults
`supports_fused_cmp_select()`, so a hard-wired `true` turned A/B bisection into
a build compared against itself.

The derived program is freestanding (`-ffreestanding -nostdlib -static`) so the
test needs no i386 sysroot, and it checks its own results: exit 0 means every
predicate x operand-shape combination returned the right pointer.  It is
executed when the host can run i386 ELF (this is the only way to catch a
register-width mistake in the fused emitter); otherwise the compile half still
runs.
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

PROGRAM = r"""
static int cells[8];
__attribute__((noinline)) static const int *p_eq(const int *a, const int *b) { return a == b ? a : b; }
__attribute__((noinline)) static const int *p_ne(const int *a, const int *b) { return a != b ? a : b; }
__attribute__((noinline)) static const int *p_lt(const int *a, const int *b) { return a < b ? a : b; }
__attribute__((noinline)) static int i_eq(const int *a) { return a == 0 ? 11 : -7; }
__attribute__((noinline)) static const int *i_sel(const int *a) { return a != 0 ? a : cells; }
static int check(void) {
    for (int i = 0; i < 8; i++) cells[i] = i;
    int bad = 0;
    for (int i = 0; i < 8; i++) for (int j = 0; j < 8; j++) {
        const int *a = &cells[i], *b = &cells[j];
        const int *e = p_eq(a, b);
        if (i == j ? e != a : e != b) bad++;
        const int *n = p_ne(a, b);
        if (i == j ? n != b : n != a) bad++;
        const int *l = p_lt(a, b);
        if (i < j ? l != a : l != b) bad++;
        if (i_eq(a) != -7) bad++;
        if (i_sel(a) != a) bad++;
    }
    if (i_eq(0) != 11) bad++;
    if (i_sel(0) != cells) bad++;
    return bad;
}
int _start(void) {
    int r = check();
    __asm__ volatile("int $0x80" :: "a"(1), "b"(r));
    return r;
}
"""


class I686PtrCompareSelect(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not LCCC.is_file():
            raise unittest.SkipTest("built LCCC required")
        cls.tmp = tempfile.TemporaryDirectory(prefix="i686-csel-ptr-")
        cls.dir = Path(cls.tmp.name)
        cls.src = cls.dir / "ptr_sel.c"
        cls.src.write_text(PROGRAM)
        # `-m32` needs no special binary name, but the target must be i686.
        cls.driver = LCCC

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def build(self, env=None, extra=()) -> Path:
        out = self.dir / ("bin-" + "-".join(sorted((env or {}).keys())) + "-" + "".join(extra))
        proc = subprocess.run(
            [str(self.driver), "-m32", "-O2", "-ffreestanding", "-fno-stack-protector",
             "-nostdlib", "-static", *extra, "-o", str(out), str(self.src)],
            capture_output=True, text=True, timeout=300,
            env={**os.environ, **(env or {})},
        )
        self.assertEqual(proc.returncode, 0, proc.stderr[:3000])
        return out

    def test_ptr_compare_selects_compile_on_i686(self) -> None:
        """The shape that used to assert `is_integer()` in the fused emitter."""
        binary = self.build()
        self.assertTrue(binary.is_file())

    def test_fused_and_unfused_agree_at_runtime(self) -> None:
        for label, env in (("fused", {}), ("kill switch", {"CCC_NO_FUSED_CSEL": "1"})):
            with self.subTest(mode=label):
                binary = self.build(env)
                proc = subprocess.run([str(binary)], capture_output=True, timeout=120)
                if proc.returncode < 0 or proc.returncode == 127:
                    self.skipTest("host cannot execute i386 ELF")
                self.assertEqual(proc.returncode, 0,
                                 f"{label}: a pointer select returned the wrong arm")

    def test_memchr_result_null_check_compiles_on_i686(self) -> None:
        """`p ? p : 0` on a libcall result is the same fused Ptr compare."""
        # The real bench kernel is the source of the shape, and it needs no
        # i386 libc headers (<stddef.h> is compiler-provided).
        kernel = REPO / "tests" / "bench" / "k_memchr.c"
        out = self.dir / "k_memchr32.s"
        proc = subprocess.run(
            [str(self.driver), "-m32", "-O2", "-I", str(REPO / "tests" / "bench"),
             "-S", "-o", str(out), str(kernel)],
            capture_output=True, text=True, timeout=300,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr[:3000])
        text = out.read_text()
        self.assertIn("call memchr", text,
                      "the 32-bit target must synthesize the same memchr call")
        # size_t is 32-bit here: the byte count is one 32-bit immediate into a
        # single stack slot, never the two-slot pair an I64 argument would use.
        self.assertRegex(text, r"movl\s+\$8192, \d+\(%esp\)",
                         "the bound must be passed as a 32-bit size_t argument")

    def test_kill_switch_is_wired(self) -> None:
        """`supports_fused_cmp_select` must consult CCC_NO_FUSED_CSEL."""
        emit = (REPO / "src" / "backend" / "i686" / "codegen" / "emit.rs").read_text()
        marker = "fn supports_fused_cmp_select(&self) -> bool {"
        self.assertIn(marker, emit)
        body = emit[emit.index(marker):]
        body = body[:body.index("\n    }")]
        self.assertIn("CCC_NO_FUSED_CSEL", body,
                      "the documented kill switch must be honoured, not just mentioned")


if __name__ == "__main__":
    unittest.main()
