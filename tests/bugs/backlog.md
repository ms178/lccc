# lccc bug backlog

Known divergences and gaps, triaged with minimal repros. Each `.c` file
in this directory compiles; the header comment records expected (GCC
oracle) vs observed (lccc) behavior. Newest entries first.

## 2026-10-03 — EDG-transplant session

### bitfield_generic_effective_type.c — E6 pin (EDGcpfe/26614)
`_Generic` on a bit field whose width matches a smaller integer type: GCC
selects the effective type (`unsigned u:8` -> `unsigned char`), lccc
selects the declared type. Feeds the E6 bit-field differential corpus.

### compound_literal_alignas.c — DR444 alignment not honored
`(_Alignas(32) int[]){...}` loses its 32-byte alignment in lccc's
emission; GCC honors it. EDG accepted the syntax (EDGcpfe/21031,21904);
the remaining work is codegen/data placement.

### Preprocessor corpus (clang-c) residual gaps — 2 of 26 executed tests
- `pragma_assume_nonnull.c`: needs the `_Nonnull` parameter qualifier
  keyword plus a `-Wnonnull` null-argument warning to reject the call.
- `pushable-diagnostics.c`: needs a real diagnostic-state machine for
  `#pragma (clang|GCC) diagnostic` (push/pop/ignore) AND a
  `-Wunused-comparison`-class warning for `1 == 1;` to trigger.
Both are multi-subsystem features; deferred in favor of corpus breadth.

### #embed packaging gap
`embed_constexpr.c` (clang-c corpus) references an auxiliary data file
that the corpus snapshot does not ship; not a compiler defect.
