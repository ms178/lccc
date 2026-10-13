# IVSR codegen fixtures (moved here from the `tests/` root, AUD-HYGIENE-1)

These are **assembly-shape fixtures**, not runnable tests: the `*.c` files are
loop shapes handed to the compiler to inspect the emitted code, and the `*.s`
files are captured outputs of earlier sessions (`*_before.s` = the pre-fix
shape).

They live outside `tests/regression/*.c` on purpose: the regression runner
compiles and RUNS every `tests/regression/*.c` and compares against GCC, which
a library-style snippet without `main` cannot satisfy.

How to use them:

```bash
target/fastbuild/lccc -O2 -S tests/regression/fixtures/ivsr/simple_loop.c -o /tmp/simple_loop.s
diff -u tests/regression/fixtures/ivsr/simple_loop_univsr.s /tmp/simple_loop.s
```

Any fixture promoted to a real gate belongs in `tests/regression/` with a
`main` and a `.flags` sidecar.
