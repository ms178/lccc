# AArch64 session 59 evidence

* `gcc-torture-500-summary.json`: machine-readable result and every remaining
  failure from the first 500 sorted GCC C torture execute tests.
* `codegen-oracle.md`: local LCCC versus ARM64 GCC 16.1, GCC trunk, and Clang
  22.1 static code-structure comparison.
* `benchmark-vm-screening.md`: randomized paired host-VM benchmark evidence.

The full raw external-suite objects and binaries are deliberately not checked
in. Reproduce them with `scripts/aarch64_execute_suite.py`; its JSON records the
exact tool versions. GNU assembler 2.47 is mandatory.
