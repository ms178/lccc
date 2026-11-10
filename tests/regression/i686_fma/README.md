# i686 FMA / F16C support fixtures

These three translation units are **support files, not standalone tests**.
They must live in a subdirectory: `tests/regression/run_regression.py`
discovers every `tests/regression/*.c` and compiles it as a single-TU test with
`-O2`, and the runner's own diagnostic says why that is deliberate
(`run_regression.py`, "the corpus's single-TU contract"):

* `i686_fma128_prefix.c` and `f16_i686_freestanding.c` each define `_start` for
  a freestanding `int $0x80` static ELF, so a root-level copy collides with any
  other such fixture -- `multiple definition of '_start'`.  That is exactly how
  the corpus went red on the commit that added them.
* `i686_fma128.c` calls the raw `__lccc_simd128_*` FMA builtins, which need
  `-mfma`; `discover()` gives a root-level `.c` `-O2` unless a `.flags` sidecar
  says otherwise, so a root-level copy failed with a permission diagnostic
  rather than testing anything.

The gates that own them pass the flags explicitly:

| fixture | gate |
|---------|------|
| `i686_fma128.c`, `i686_fma128_prefix.c` | `check_i686_fma128.sh` |
| `f16_i686_freestanding.c` | `check_f16_conversion_parity.sh` (i686 leg) |

Keeping them out of the corpus root is the fix, not a suppression: the gates
run them, and they run the executed-value differentials the corpus runner
cannot express (native i386 execution, pre-fix non-vacuity legs).
